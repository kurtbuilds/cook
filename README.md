# Cook

Cook is a declarative tool for managing infrastructure. It is alternative to ansible, saltstack, terraform, pulumi, and others.

Let's confirm we can connect to a remote host.

```bash
cook ssh -H user@host echo hello world
```

Let's add the host.

```bash
echo "host user@host" > Cookfile
```

Now let's add a user.

```bash
echo "user michaeljackson" >> Cookfile
```

Now let's apply the changes.

```bash
cook up
```

You've now configured the server!

Cook runs independent rules concurrently over one SSH connection. If the server
refuses new sessions, Cook warns once and automatically queues session openings.
See [SSH concurrency](docs/ssh.md).

Here are some other common commands:

Run a rule as a one-off:

```bash
cook run package postgresql
```

## Restarting services

A running service restarts when its unit file changes, or when a rule named
in its `restart_on` applies a change in the same run:

```kdl
cp config/vector.yaml /etc/vector/vector.yaml
service vector restart_on="file:/etc/vector/vector.yaml"
```

`restart_on` takes space-separated rule references, like `requires`, and
orders those rules first. The unit file argument is optional. Without it, cook
does not write or enable the unit (a package already did) and only manages its
restarts. A stopped service is not restarted.

## Removing services

Cook keeps no record of what it applied, so deleting a `service` line leaves
its units on the host. To remove them, replace the line with a tombstone:

```kdl
tombstone service update-market-close
```

Cook stops and disables the service and its timer, deletes their unit files
from `/etc/systemd/system`, and reloads systemd. When neither file exists, the
rule does nothing, so the line can stay until every host has applied it.
Binaries, working directories, and state directories stay; remove them
separately. A config cannot have both `service foo` and `tombstone service foo`.
Only `service` can be tombstoned for now.


## Installing the daemon

By default, cook will detect the operating system
