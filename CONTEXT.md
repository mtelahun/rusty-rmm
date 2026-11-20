# RustyRMM

Remote monitoring and management: a central service observes and acts on many managed machines through software installed on each.

## Language

**Endpoint**:
A managed machine (Windows, Linux or macOS) known to the Server.
_Avoid_: Host, device, node

**Agent**:
The RustyRMM software installed and running on an Endpoint.
_Avoid_: Client

**Server**:
The central RustyRMM management service that Agents connect to.
_Avoid_: Backend, master

**Operator**:
A human who uses the Server to observe and act on Endpoints.
_Avoid_: User, admin

**Environment**:
A named group of Endpoints, such as a customer or site, on a single Server shared by all Operators. Not a security boundary.
_Avoid_: Tenant, organization, group

**Pairing**:
Associating a new Endpoint with an Environment by installing the Agent from an Environment-specific link.
_Avoid_: Enrollment, onboarding

**Session**:
One Agent's live connection to the Server, from its hello until disconnect. A newer Session for the same Agent replaces the old one.
_Avoid_: Connection, stream

**Online**:
The state of an Endpoint whose Agent has a live Session with recent heartbeats.
_Avoid_: Connected, up

**Command**:
An Operator's request to run something on an Endpoint, ending in a result.
_Avoid_: Job, task, action

**Sample**:
One point-in-time Monitoring measurement from an Endpoint.
_Avoid_: Metric, datapoint, reading

**Pairing Token**:
A reusable, expiring secret tied to one Environment that authorizes Pairing. Operators can revoke it.
_Avoid_: Enrollment key, invite

**Revoked**:
The state of an Endpoint whose Agent is no longer allowed to open a Session.
_Avoid_: Disabled, banned, blocked

**Tunnel**:
An Operator-opened relay from a local port of the CLI, through the Server, to a port on an Endpoint's loopback. It lasts as long as the CLI process, and only Operators with the Tunnel permission can open one.
_Avoid_: Proxy, port forward

**Inventory**:
The descriptive snapshot of an Endpoint: hardware, OS, network and update status. Contrast with a Sample, which is a point-in-time measurement.
_Avoid_: Endpoint Information, facts, specs
