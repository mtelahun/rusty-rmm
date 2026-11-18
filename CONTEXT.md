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
