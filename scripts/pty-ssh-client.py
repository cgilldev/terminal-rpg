#!/usr/bin/env python3
"""Drive an SSH smoke client through a real, resizable pseudo-terminal."""

import argparse
import errno
import fcntl
import os
import pty
import select
import signal
import struct
import sys
import termios
import time


READY_MARKER = b"GRAVE KNIGHT"


def set_size(fd: int, columns: int, rows: int) -> None:
    size = struct.pack("HHHH", rows, columns, 0, 0)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, size)


def read_chunk(fd: int, output: bytearray, timeout: float) -> bool:
    readable, _, _ = select.select([fd], [], [], timeout)
    if not readable:
        return True
    try:
        chunk = os.read(fd, 65536)
    except OSError as error:
        if error.errno == errno.EIO:
            return False
        raise
    if not chunk:
        return False
    output.extend(chunk)
    return True


def read_until(fd: int, output: bytearray, marker: bytes, timeout: float) -> None:
    deadline = time.monotonic() + timeout
    while marker not in output:
        if time.monotonic() >= deadline or not read_chunk(fd, output, 0.05):
            raise TimeoutError(f"SSH client did not render {marker!r}")


def read_activity(fd: int, output: bytearray, timeout: float = 2.0) -> None:
    """Wait for output, then drain it until the stream is briefly quiet."""
    initial = len(output)
    deadline = time.monotonic() + timeout
    quiet_since = None
    while time.monotonic() < deadline:
        before = len(output)
        if not read_chunk(fd, output, 0.05):
            return
        if len(output) > before:
            quiet_since = time.monotonic()
        if len(output) > initial and quiet_since is not None:
            if time.monotonic() - quiet_since >= 0.1:
                return
    raise TimeoutError("SSH client produced no settled output after an action")


def drain_until_quiet(fd: int, output: bytearray, timeout: float = 1.0) -> None:
    deadline = time.monotonic() + timeout
    last_data = time.monotonic()
    while time.monotonic() < deadline:
        before = len(output)
        if not read_chunk(fd, output, 0.05):
            return
        if len(output) > before:
            last_data = time.monotonic()
        elif time.monotonic() - last_data >= 0.15:
            return


def drain_child(fd: int, child: int, output: bytearray, timeout: float) -> int:
    """Drain through PTY EOF/EIO, even when the child exits first."""
    deadline = time.monotonic() + timeout
    output_open = True
    status = None
    while time.monotonic() < deadline and (output_open or status is None):
        if output_open:
            output_open = read_chunk(fd, output, 0.05)
        elif status is None:
            time.sleep(0.01)
        if status is None:
            waited, candidate = os.waitpid(child, os.WNOHANG)
            if waited == child:
                status = candidate
    if status is None:
        os.kill(child, signal.SIGKILL)
        _, status = os.waitpid(child, 0)
        drain_ready(fd, output)
    return status


def drain_ready(fd: int, output: bytearray) -> None:
    while select.select([fd], [], [], 0)[0]:
        if not read_chunk(fd, output, 0):
            break


def parse_size(value: str) -> tuple[int, int]:
    try:
        columns, rows = value.lower().split("x", 1)
        parsed = (int(columns), int(rows))
    except (ValueError, TypeError) as error:
        raise argparse.ArgumentTypeError("size must be COLUMNSxROWS") from error
    if parsed[0] <= 0 or parsed[1] <= 0:
        raise argparse.ArgumentTypeError("size dimensions must be positive")
    return parsed


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", required=True)
    parser.add_argument("--action", default="")
    parser.add_argument("--fragment", action="append", default=[])
    parser.add_argument("--columns", type=int, default=80)
    parser.add_argument("--rows", type=int, default=24)
    parser.add_argument("--resize", action="append", type=parse_size, default=[])
    parser.add_argument("--spam", type=int, default=0)
    parser.add_argument("--stall", action="store_true")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("a command is required after --")

    child, master = pty.fork()
    if child == 0:
        os.execvp(command[0], command)

    output = bytearray()
    status = None
    try:
        set_size(master, args.columns, args.rows)
        read_until(master, output, READY_MARKER, 4.0)
        drain_until_quiet(master, output)

        if args.spam:
            os.set_blocking(master, False)
            payload = b"s" * 64
            for _ in range(args.spam):
                try:
                    os.write(master, payload)
                except BlockingIOError:
                    break
                time.sleep(0.001)
            time.sleep(0.5)
            if args.stall:
                os.kill(child, signal.SIGKILL)
                _, status = os.waitpid(child, 0)
                drain_ready(master, output)
                return_code = 0
            else:
                os.set_blocking(master, True)
                os.write(master, b"Q")
                status = drain_child(master, child, output, 5.0)
                return_code = os.waitstatus_to_exitcode(status)
        else:
            if args.action:
                before = len(output)
                os.write(master, args.action.encode())
                read_activity(master, output)
                if len(output) == before:
                    raise TimeoutError("action produced no output")
            for fragment in args.fragment:
                os.write(master, fragment.encode())
                time.sleep(0.005)
                read_chunk(master, output, 0.001)
            if args.fragment:
                drain_until_quiet(master, output)
            for columns, rows in args.resize:
                set_size(master, columns, rows)
                read_activity(master, output)
            os.write(master, b"Q")
            status = drain_child(master, child, output, 5.0)
            return_code = os.waitstatus_to_exitcode(status)
    except (OSError, TimeoutError) as error:
        print(error, file=sys.stderr)
        try:
            os.kill(child, signal.SIGKILL)
        except ProcessLookupError:
            pass
        try:
            os.waitpid(child, 0)
        except ChildProcessError:
            pass
        return_code = 1
    finally:
        os.close(master)
        with open(args.output, "wb") as transcript:
            transcript.write(output)

    return return_code


if __name__ == "__main__":
    sys.exit(main())
