# rustyrmm-cli

The `rustyrmm` Operator command-line interface.

## Operators

```sh
rustyrmm operator add <username> <email>     # Admins only; prompts for the password
rustyrmm operator email <username> <email>   # your own address, or anyone's as an Admin
rustyrmm operator list                       # Admins only; includes each email
```

## Update Installs

Linux Endpoints (apt, dnf) only; macOS Endpoints are refused. The Endpoint is never rebooted.

```sh
rustyrmm updates install <endpoint>              # all pending Updates; prints the record
rustyrmm updates install <endpoint> --security   # only security Updates
rustyrmm updates install <endpoint> --wait       # wait, print each Update's result; exit 0 only if Succeeded
rustyrmm updates installs [<endpoint>]           # Update Installs, newest first
```

An Offline Endpoint's request ends Skipped (Offline), and one made while another is Requested
or Running ends Rejected (busy). Others end Succeeded, Failed or Interrupted.
