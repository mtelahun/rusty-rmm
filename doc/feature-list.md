# Open MM Feature List

## Pairing
Download client installer from specially crafted url to associate an endpoint with a specific environment

## Tunneling
Create local or remote tunnels through an endpoint. A local tunnel talks to a port on the endpoint. A remote tunnel talks to a host on a network reachable by the endpoint but not directly accessable from the management server.
- Local tunnels: done (`rustyrmm tunnel open`; needs the per-Operator Tunnel permission).
- Remote tunnels: planned. Limited to hosts on the Endpoint's own subnets or an allow list.

## Endpoint Information
Get information about the Endpoint:
- id/uuid
- hostname
- Timezone
- OS - full name, family, version, OS Virtualization system, OS Virtualization Role
- CPU - family, model, core count, thread count
- Memory - total, usage
- Per Disk - total, usage, io usage
- IP addresses
- Update status - normal updates, security updates
- client version

## Installed Software

## Monitoring
real-time monitoring:
- CPU usage
- Memory usage
- Network usage
- process list - PID, name, PPID, state, memory usage, cpu usage, disk usage, net usage, command line