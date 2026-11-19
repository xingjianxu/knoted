"""Let the actual ASGI server and its namespace init finish before systemd cleanup."""
import os
from pathlib import Path
import select
import signal
import sys

# systemd supplies the main PID in the manager's namespace to control commands.
# A pidfd pins the task while we wait for it or send TERM.
if len(sys.argv) > 1:
    main_pid = int(sys.argv[1])
    try:
        server = os.pidfd_open(main_pid)
        status = Path(f"/proc/{main_pid}/status").read_text().splitlines()
        parent = int(next(line.split()[1] for line in status if line.startswith("PPid:")))
        init = os.pidfd_open(parent)
        own_group = Path("/proc/self/cgroup").read_bytes()
        if any(Path(f"/proc/{pid}/cgroup").read_bytes() != own_group for pid in (main_pid, parent)):
            sys.exit("refusing to signal a process outside this service")
        signal.pidfd_send_signal(server, signal.SIGTERM)
        poll = select.poll()
        poll.register(init, select.POLLIN)
        poll.poll()
    except (ProcessLookupError, FileNotFoundError):
        pass
