#!/usr/bin/env python3
"""Smoke-test local play and terminal restoration through a real PTY."""

import argparse
import errno
import fcntl
import os
import re
import select
import signal
import struct
import sys
import termios
import time


ENTER_ALTERNATE_SCREEN = b"\x1b[?1049h"
LEAVE_ALTERNATE_SCREEN = b"\x1b[?1049l"
HIDE_CURSOR = b"\x1b[?25l"
SHOW_CURSOR = b"\x1b[?25h"
SGR_PATTERN = re.compile(rb"\x1b\[([0-9;]*)m")


def contains_color_sgr(output: bytes) -> bool:
    for match in SGR_PATTERN.finditer(output):
        parameters = [int(value) for value in match.group(1).split(b";") if value]
        if any(
            value in (38, 48)
            or 30 <= value <= 37
            or 40 <= value <= 47
            or 90 <= value <= 107
            for value in parameters
        ):
            return True
    return False


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
            raise TimeoutError(f"local game did not render {marker!r}")


def wait_for_exit(fd: int, child: int, output: bytearray, timeout: float) -> int:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        read_chunk(fd, output, 0.05)
        waited, status = os.waitpid(child, os.WNOHANG)
        if waited == child:
            return status
    raise TimeoutError("local game did not exit after quit")


def drain_to_eof(fd: int, output: bytearray) -> None:
    deadline = time.monotonic() + 1.0
    while time.monotonic() < deadline and read_chunk(fd, output, 0.05):
        pass


def main() -> int:
    parser = argparse.ArgumentParser()
    expectation = parser.add_mutually_exclusive_group()
    expectation.add_argument("--expect-color", action="store_true")
    expectation.add_argument("--expect-no-color", action="store_true")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("a game command is required after --")

    master, slave = os.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
    original_attributes = termios.tcgetattr(slave)
    child = os.fork()
    if child == 0:
        os.close(master)
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        for target in (0, 1, 2):
            os.dup2(slave, target)
        if slave > 2:
            os.close(slave)
        if args.expect_color:
            os.environ.pop("NO_COLOR", None)
            os.environ.setdefault("TERM", "xterm-256color")
        os.execvp(command[0], command)

    output = bytearray()
    status = None
    slave_open = True
    try:
        read_until(master, output, b"TERMINAL", 5.0)
        os.write(master, b"\r")
        read_until(master, output, b"GRAVE", 5.0)
        os.write(master, b"Q")
        status = wait_for_exit(master, child, output, 5.0)

        restored_attributes = termios.tcgetattr(slave)
        if restored_attributes != original_attributes:
            raise RuntimeError("local game did not restore terminal attributes")
        os.close(slave)
        slave_open = False
        drain_to_eof(master, output)

        if os.waitstatus_to_exitcode(status) != 0:
            raise RuntimeError("local game exited unsuccessfully")
        enter = output.find(ENTER_ALTERNATE_SCREEN)
        hide = output.find(HIDE_CURSOR, enter + 1)
        show = output.rfind(SHOW_CURSOR)
        leave = output.rfind(LEAVE_ALTERNATE_SCREEN)
        if not (0 <= enter < hide < show < leave):
            raise RuntimeError("alternate-screen or cursor restoration sequence is incomplete")
        has_color = contains_color_sgr(bytes(output))
        if args.expect_color and not has_color:
            raise RuntimeError("colored local game emitted no ANSI colors")
        if args.expect_no_color and has_color:
            raise RuntimeError("--no-color local game emitted ANSI colors")
    except (OSError, RuntimeError, TimeoutError) as error:
        print(error, file=sys.stderr)
        if status is None:
            try:
                os.kill(child, signal.SIGKILL)
            except ProcessLookupError:
                pass
            try:
                os.waitpid(child, 0)
            except ChildProcessError:
                pass
        return 1
    finally:
        os.close(master)
        if slave_open:
            os.close(slave)

    print("Local terminal verification passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
