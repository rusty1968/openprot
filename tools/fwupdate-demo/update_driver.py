#!/usr/bin/env python3
"""Off-box demo driver: trigger an OpenPRoT external-staging update via Redfish
and survive the BMC activation blackout.

Roles:
  - This script is the OFF-BOX actor (runs on a laptop, not the BMC).
  - It delivers the image + triggers the on-BMC Update Agent via a Redfish
    action, then tolerates the BMC disappearing during activation and reports
    the outcome once it returns.

It does NOT speak PLDM. The on-BMC agent does the staging write and the PLDM
exchange with OpenPRoT; this script only drives Redfish and reads results back
through it. The agent's stack is out of scope here (it may reuse OpenBMC's C/C++
pldmd, or be a Rust agent built on OpenPRoT's pldm-lib/pldm-common).

See docs/src/design/demo-proposal-flow.md for the full flow this drives.
"""

import argparse
import os
import sys
import time

import requests
from requests.exceptions import ConnectionError, Timeout

# Connection failures we treat as the EXPECTED activation blackout, not errors.
OUTAGE_EXCEPTIONS = (ConnectionError, Timeout)


def make_session(user, password, ca_bundle, insecure):
    s = requests.Session()
    s.auth = (user, password)
    s.headers.update({"OData-Version": "4.0"})
    if insecure:
        # BMCs often ship self-signed certs. Acceptable on an isolated demo
        # network only; never against production. Warn loudly.
        print("WARNING: TLS verification disabled (--insecure).", file=sys.stderr)
        s.verify = False
        requests.packages.urllib3.disable_warnings()  # silence single warning
    else:
        s.verify = ca_bundle or True
    return s


def trigger_update(session, base, image_path, targets, req_timeout):
    """Deliver the image and start the update via the Redfish multipart push
    action. Returns a task-monitor URL to poll.

    Alternative: UpdateService.SimpleUpdate with an ImageURI the BMC pulls —
    swap this function if the image is served over HTTP/TFTP instead of pushed.
    """
    url = f"{base}/redfish/v1/UpdateService/update"  # multipart push target
    with open(image_path, "rb") as fh:
        files = {"UpdateFile": (os.path.basename(image_path), fh,
                                "application/octet-stream")}
        # Optional: scope which component/target the agent should activate.
        data = {}
        if targets:
            data["Targets"] = targets
        resp = session.post(url, files=files, data=data, timeout=req_timeout)

    if resp.status_code not in (200, 202):
        raise SystemExit(f"Update trigger rejected: {resp.status_code} {resp.text}")

    # Async updates return a Task; follow its monitor (Location header preferred).
    monitor = resp.headers.get("Location")
    if not monitor:
        try:
            monitor = resp.json().get("@odata.id")
        except ValueError:
            monitor = None
    if not monitor:
        raise SystemExit("No task monitor returned; cannot track activation.")
    return monitor if monitor.startswith("http") else base + monitor


def poll_through_outage(session, monitor, activation_timeout, req_timeout, poll_every):
    """Poll the task. Connection failures during activation are EXPECTED (the
    BMC is powered down and Redfish is offline), so we retry rather than abort,
    up to activation_timeout (this is the off-box analogue of
    EstimatedTimeForActivation). Returns the final task JSON.
    """
    deadline = time.monotonic() + activation_timeout
    saw_outage = False

    while time.monotonic() < deadline:
        try:
            r = session.get(monitor, timeout=req_timeout)
            if r.status_code >= 500:
                raise ConnectionError(f"server {r.status_code}")  # treat as outage
            task = r.json()
            state = task.get("TaskState", "Unknown")

            if saw_outage:
                print("  BMC is back online.")
                saw_outage = False

            if state in ("Completed", "Exception", "Killed", "Cancelled"):
                return task
            print(f"  activation in progress... (TaskState={state})")

        except OUTAGE_EXCEPTIONS:
            if not saw_outage:
                print("  BMC offline (activation in progress) — waiting for it "
                      "to come back...")
                saw_outage = True
        time.sleep(poll_every)

    raise SystemExit("Timed out waiting for the BMC to return from activation.")


def read_active_version(session, base, req_timeout):
    """After-snapshot: read the active firmware version from FirmwareInventory.
    Illustrative — the exact inventory id depends on the platform.
    """
    url = f"{base}/redfish/v1/UpdateService/FirmwareInventory"
    try:
        coll = session.get(url, timeout=req_timeout).json()
        versions = []
        for member in coll.get("Members", []):
            item = session.get(base + member["@odata.id"], timeout=req_timeout).json()
            versions.append(f'{item.get("Id")}={item.get("Version")}')
        return ", ".join(versions) or "(none reported)"
    except (ValueError, KeyError, *OUTAGE_EXCEPTIONS):
        return "(inventory unavailable)"


def main():
    ap = argparse.ArgumentParser(description="Off-box Redfish driver for the "
                                             "OpenPRoT external-staging demo.")
    ap.add_argument("--bmc", required=True, help="https://<bmc-host>")
    ap.add_argument("--image", required=True, help="path to the candidate image")
    ap.add_argument("--targets", nargs="*", default=None,
                    help="optional Redfish Target @odata.ids to scope the update")
    ap.add_argument("--activation-timeout", type=float, default=300.0,
                    help="max seconds to wait across the blackout "
                         "(off-box analogue of EstimatedTimeForActivation)")
    ap.add_argument("--request-timeout", type=float, default=10.0)
    ap.add_argument("--poll-every", type=float, default=3.0)
    ap.add_argument("--ca-bundle", default=os.environ.get("BMC_CA_BUNDLE"))
    ap.add_argument("--insecure", action="store_true",
                    help="disable TLS verification (demo networks only)")
    args = ap.parse_args()

    # Credentials come from the environment, never the command line / source.
    user = os.environ.get("BMC_USER")
    password = os.environ.get("BMC_PASS")
    if not user or not password:
        raise SystemExit("Set BMC_USER and BMC_PASS in the environment.")

    base = args.bmc.rstrip("/")
    session = make_session(user, password, args.ca_bundle, args.insecure)

    print(f"[before] active firmware: {read_active_version(session, base, args.request_timeout)}")

    print("Sending update (deliver + trigger via Redfish)...")
    monitor = trigger_update(session, base, args.image, args.targets,
                             args.request_timeout)
    print(f"  tracking task: {monitor}")

    task = poll_through_outage(session, monitor, args.activation_timeout,
                               args.request_timeout, args.poll_every)

    state = task.get("TaskState")
    status = task.get("TaskStatus", "?")
    print(f"[result] TaskState={state} TaskStatus={status}")
    for msg in task.get("Messages", []):
        print(f"    {msg.get('Message', msg)}")

    print(f"[after] active firmware: {read_active_version(session, base, args.request_timeout)}")
    # Task success alone does not prove the trial boot committed; the
    # after-snapshot is the proof.
    sys.exit(0 if state == "Completed" and status in ("OK", "?") else 1)


if __name__ == "__main__":
    main()
