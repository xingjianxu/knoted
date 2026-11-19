"""PID namespace init. Keep guardians alive after systemd's main process exits."""
import array
import os
import socket
import sys

# PID 1 must stay until the last adopted guardian and its children are reaped.
if os.getpid() != 1:
    sys.exit("knoted-intelligence requires a private PID namespace")
notify = os.environ.pop("NOTIFY_SOCKET")
pid = os.fork()
if pid == 0:
    os.execv(sys.argv[1], sys.argv[1:])

# Publish the actual server as MainPID, even though its PID numbers differ on
# the two sides of the namespace. systemd directs KillMode=mixed's TERM there.
with socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM) as client:
    fd = os.pidfd_open(pid)
    address = "\0" + notify[1:] if notify.startswith("@") else notify
    try:
        client.sendmsg([b"MAINPIDFD=1\nREADY=1"],
                       [(socket.SOL_SOCKET, socket.SCM_RIGHTS, array.array("i", [fd]))],
                       0, address)
    finally:
        os.close(fd)

while True:
    try:
        os.wait()
    except ChildProcessError:
        break
