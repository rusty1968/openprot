## Architecture Description

This diagram illustrates the **Hybrid Direct Ownership** architecture for MCTP integration in Hubris, demonstrating how Platform Root of Trust (PRoT) systems can achieve optimal performance and security through selective resource management strategies.

### **System Overview**

The architecture is divided into two distinct domains:

1. **MCTP Domain** - High-performance, security-critical communication using direct hardware ownership
2. **Traditional I2C Domain** - Backward-compatible, shared resource model for existing functionality

### **External Components**

**BMC/Management Controller** (Orange)
- Represents the external Baseboard Management Controller
- Communicates with the Hubris PRoT system via I2C MCTP protocol
- Connects to both primary and backup MCTP buses for redundancy

### **MCTP Domain Components**

#### **Application Layer** (Blue)
- **PLDM Task**: Handles Platform Level Data Model operations including firmware updates, sensor readings, and platform management
- **SPDM Task**: Manages Security Protocol and Data Model for attestation, measurement, and secure communication
- **Vendor Task**: Processes vendor-specific MCTP protocols and custom management functions

#### **MCTP Protocol Layer** (Blue)
- **MCTP Control Task**: 
  - Manages MCTP endpoint discovery and configuration
  - Handles MCTP control messages (Set/Get Endpoint ID, etc.)
  - Software-only task with no direct hardware ownership
  - Communicates with MCTP Router Task via IPC

- **MCTP Router Task**:
  - **Primary responsibility**: Direct ownership of I2C controllers 2 and 3
  - **Transport layer**: Handles raw I2C MCTP packet transmission and reception
  - **Protocol layer**: Routes incoming MCTP messages to appropriate application tasks
  - **Performance critical**: Provides sub-10μs latency for real-time security protocols

#### **MCTP Transport Layer** (Green)
- **I2C Controller 2**: Dedicated MCTP Bus A - primary secure communication channel
- **I2C Controller 3**: Dedicated MCTP Bus B - backup/redundant secure communication channel
- Both controllers are exclusively owned by the MCTP Router Task

### **Traditional I2C Domain Components**

#### **Traditional I2C Layer** (Purple)
- **I2C Server Task**: 
  - Implements the shared resource model
  - Manages multiple I2C controllers for backward compatibility
  - Provides IPC-based access to I2C resources

- **I2C Hardware Controllers**:
  - **I2C Controller 4**: Sensors and PMBus devices
  - **I2C Controller 7**: FRU (Field Replaceable Unit) and expansion interfaces

#### **Sensor Management** (Purple)
- **Sensor Manager**: Collects environmental and system sensor data
- **Thermal Monitor**: Manages thermal policies and fan control
- Both tasks access I2C hardware through the I2C Server Task via IPC

### **Communication Flows**

#### **MCTP Communication Flow** (Dotted lines)
1. BMC initiates I2C MCTP communication on primary bus (I2C Controller 2)
2. Backup communication available on secondary bus (I2C Controller 3)
3. MCTP Router Task receives raw I2C packets directly from hardware

#### **MCTP Protocol Flow** (Solid arrows)
1. **Raw packet processing**: MCTP Router Task processes incoming I2C packets
2. **Message routing**: Router determines destination based on MCTP message type
3. **Application delivery**: Routed messages delivered to PLDM, SPDM, or Vendor tasks
4. **Control message handling**: MCTP control messages routed to MCTP Control Task

#### **Traditional I2C Flow** (Solid arrows)
1. **IPC requests**: Sensor Manager and Thermal Monitor send I2C requests via IPC
2. **Server processing**: I2C Server Task handles requests and accesses hardware
3. **Hardware operation**: Server controls I2C Controllers 4 and 7 for sensor operations

### **Key Architectural Benefits**

#### **Performance Optimization**
- **Direct hardware access** for MCTP eliminates IPC overhead 
- **Dedicated buses** prevent contention between security-critical MCTP and routine sensor traffic
- **Hardware redundancy** with dual MCTP buses ensures communication reliability

#### **Security Isolation**
- **Complete separation** between MCTP security domain and traditional I2C operations
- **No shared resources** between security-critical and general-purpose functions
- **Attack surface reduction** through hardware-enforced boundaries

#### **Maintainability**
- **Backward compatibility** preserved for existing I2C sensor infrastructure
- **Clear separation of concerns** between performance-critical and general-purpose operations
- **Incremental deployment** allows gradual migration strategies

#### **Resource Efficiency**
- **Optimal resource allocation** based on performance and security requirements
- **Reuse of existing infrastructure** for non-critical operations
- **Scalable architecture** supporting future protocol additions

This hybrid approach represents the optimal balance between performance, security, and implementation complexity for MCTP integration in Hubris-based PRoT systems.

![MCTP Architecture Diagram](mctp-architecture.svg)

