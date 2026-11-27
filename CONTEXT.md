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
A human who uses the Server to observe and act on Endpoints. Has an email address (so the Server can email Notifications). Belongs to the Administrator group or not, and to zero or more Environments.
_Avoid_: User, admin

**Administrator group (Admin group)**:
The built-in group of Operators who access every Environment and manage Operators, Environments and membership. The first Operator is an Admin; the last Admin can't be removed.
_Avoid_: Root, superuser

**Environment**:
A named group of Endpoints, such as a customer or site, on a single Server. An access boundary: Administrators access every Environment; other Operators only the ones they belong to. May have an optional free-text owner (the customer or site it belongs to), which is a label only and is visible and editable only to those who can access the Environment.
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

**Alert**:
An Endpoint's offline condition: opened when it has been Offline past its threshold, resolved when it is Online again. At most one is open per Endpoint, so one outage is one Alert. Has a severity (informational, warning or error; offline Alerts are warnings). An Operator can snooze an open Alert for 12 hours (it leaves the unhandled count, not the Alert), resolve it by hand (the Endpoint then gets no new Alert until it has been seen Online), and attach action notes: short one-line comments on what was done.
_Avoid_: Incident, alarm

**Notification**:
A message to one Operator through one channel (the web UI, email, or both, chosen per recipient) about an Alert opening or resolving. Operators in the Environment's recipient list who can access it each get their own; email batches one check's changes in an Environment into a single digest.
_Avoid_: Alert (that is the condition, not the message)

**Revoked**:
The state of an Endpoint whose Agent is no longer allowed to open a Session.
_Avoid_: Disabled, banned, blocked

**Update**:
One pending operating-system update on an Endpoint (a Linux package upgrade or a Windows update), which may be a security Update. Patch management is the feature that shows and installs them.
_Avoid_: Patch, package

**Inventory**:
The descriptive snapshot of an Endpoint: hardware, OS, network and update status. Contrast with a Sample, which is a point-in-time measurement.
_Avoid_: Endpoint Information, facts, specs
