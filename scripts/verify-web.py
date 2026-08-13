#!/usr/bin/env python3
"""Live HTTP/WebSocket smoke for the browser terminal transport."""

import argparse
import base64
import hashlib
import json
import os
import re
import select
import socket
import struct
import time
import urllib.request


ANSI = re.compile(rb"\x1b\[[0-?]*[ -/]*[@-~]")
WEBSOCKET_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"


class WebSocketClient:
    def __init__(self, host: str, port: int, fragmented_hello: bool = False):
        self.host = host
        self.port = port
        self.socket = socket.create_connection((host, port), timeout=4)
        self.socket.settimeout(4)
        self.output = bytearray()
        key = base64.b64encode(os.urandom(16)).decode()
        request = (
            f"GET /ws HTTP/1.1\r\n"
            f"Host: {host}:{port}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n"
            f"Origin: http://{host}:{port}\r\n\r\n"
        ).encode()
        self.socket.sendall(request)
        response = self._read_headers()
        if not response.startswith(b"HTTP/1.1 101"):
            raise RuntimeError(f"WebSocket upgrade failed: {response[:120]!r}")
        expected = base64.b64encode(
            hashlib.sha1((key + WEBSOCKET_GUID).encode()).digest()
        )
        if b"sec-websocket-accept: " + expected.lower() not in response.lower():
            raise RuntimeError("WebSocket accept key was invalid")
        hello = json.dumps(
            {"v": 1, "type": "hello", "cols": 80, "rows": 24},
            separators=(",", ":"),
        ).encode()
        if fragmented_hello:
            midpoint = len(hello) // 2
            self.send_frame(1, hello[:midpoint], final=False)
            self.send_frame(0, hello[midpoint:], final=True)
        else:
            self.send_frame(1, hello)
        self.seed = self._wait_ready()
        self.wait_for_screen("GRAVE KNIGHT")

    def _read_headers(self) -> bytes:
        response = bytearray()
        while b"\r\n\r\n" not in response:
            chunk = self.socket.recv(4096)
            if not chunk:
                raise RuntimeError("connection closed during HTTP upgrade")
            response.extend(chunk)
        return bytes(response)

    def send_frame(self, opcode: int, payload: bytes, final: bool = True) -> None:
        first = opcode | (0x80 if final else 0)
        length = len(payload)
        if length < 126:
            header = bytes((first, 0x80 | length))
        elif length <= 65535:
            header = bytes((first, 0x80 | 126)) + struct.pack("!H", length)
        else:
            header = bytes((first, 0x80 | 127)) + struct.pack("!Q", length)
        mask = os.urandom(4)
        masked = bytes(value ^ mask[index % 4] for index, value in enumerate(payload))
        self.socket.sendall(header + mask + masked)

    def send_json(self, message: dict) -> None:
        self.send_frame(1, json.dumps({"v": 1, **message}, separators=(",", ":")).encode())

    def receive_frame(self) -> tuple[int, bytes]:
        first, second = self._read_exact(2)
        opcode = first & 0x0F
        length = second & 0x7F
        if length == 126:
            length = struct.unpack("!H", self._read_exact(2))[0]
        elif length == 127:
            length = struct.unpack("!Q", self._read_exact(8))[0]
        if second & 0x80:
            raise RuntimeError("server frames must not be masked")
        return opcode, self._read_exact(length)

    def _read_exact(self, length: int) -> bytes:
        data = bytearray()
        while len(data) < length:
            chunk = self.socket.recv(length - len(data))
            if not chunk:
                raise RuntimeError("WebSocket closed unexpectedly")
            data.extend(chunk)
        return bytes(data)

    def _wait_ready(self) -> int:
        deadline = time.monotonic() + 4
        while time.monotonic() < deadline:
            opcode, payload = self.receive_frame()
            if opcode == 1:
                message = json.loads(payload)
                if message.get("type") == "ready" and message.get("v") == 1:
                    return int(message["seed"])
                if message.get("type") == "error":
                    raise RuntimeError(message.get("message", "protocol error"))
            elif opcode == 2:
                self.output.extend(payload)
        raise TimeoutError("browser session never became ready")

    def screen_text(self) -> str:
        return ANSI.sub(b"", bytes(self.output)).decode("utf-8", "replace")

    def wait_for_screen(self, marker: str) -> None:
        deadline = time.monotonic() + 4
        while marker not in self.screen_text():
            if time.monotonic() >= deadline:
                raise TimeoutError(f"terminal did not render {marker!r}")
            try:
                opcode, payload = self.receive_frame()
            except TimeoutError as error:
                tail = self.screen_text()[-1_000:]
                raise TimeoutError(f"terminal did not render {marker!r}; recent output: {tail!r}") from error
            if opcode == 2:
                self.output.extend(payload)
            elif opcode == 1:
                message = json.loads(payload)
                if message.get("type") == "error":
                    raise RuntimeError(message.get("message", "protocol error"))
            elif opcode == 8:
                raise RuntimeError("session closed before expected output")

    def resize(self, columns: int, rows: int) -> None:
        self.send_json({"type": "resize", "cols": columns, "rows": rows})

    def input(self, data: str) -> None:
        self.send_json({"type": "input", "data": data})

    def reset_capture(self) -> None:
        self.output.clear()

    def drain_output(self) -> None:
        """Consume redraw frames until the server is briefly quiet."""
        quiet_deadline = time.monotonic() + 0.15
        while time.monotonic() < quiet_deadline:
            readable, _, _ = select.select([self.socket], [], [], 0.03)
            if not readable:
                continue
            opcode, payload = self.receive_frame()
            if opcode == 2:
                self.output.extend(payload)
            elif opcode == 1:
                message = json.loads(payload)
                if message.get("type") == "error":
                    raise RuntimeError(message.get("message", "protocol error"))
            elif opcode == 8:
                raise RuntimeError("session closed during redraw")
            quiet_deadline = time.monotonic() + 0.08

    def wait_for_close(self) -> None:
        deadline = time.monotonic() + 4
        while time.monotonic() < deadline:
            opcode, payload = self.receive_frame()
            if opcode == 8:
                if payload and struct.unpack("!H", payload[:2])[0] != 1000:
                    raise RuntimeError("game did not close normally")
                return
            if opcode == 2:
                self.output.extend(payload)
        raise TimeoutError("browser session did not close")

    def close_abruptly(self) -> None:
        self.socket.close()


def fetch_page(url: str) -> None:
    with urllib.request.urlopen(url, timeout=4) as response:
        body = response.read()
        if response.status != 200 or b'id="terminal"' not in body:
            raise RuntimeError("browser page was unavailable")
        if "default-src 'none'" not in response.headers.get("Content-Security-Policy", ""):
            raise RuntimeError("browser page lacked its restrictive CSP")


def verify_touch_ui(url: str) -> None:
    with urllib.request.urlopen(url, timeout=4) as response:
        page = response.read()
    base = url.rsplit("/", 1)[0]
    with urllib.request.urlopen(f"{base}/app.js", timeout=4) as response:
        script = response.read()
    with urllib.request.urlopen(f"{base}/app.css", timeout=4) as response:
        styles = response.read()
    required_page = (b'id="touch-controls"', b'data-input="q"', b'data-input="tab"', b'data-input="enter"', b'data-input="escape"')
    required_script = (b"navigator.maxTouchPoints", b"pointer: coarse", b"function sendInput", b"function touchInput", b"touchControls.addEventListener")
    required_styles = (b".touch-pad", b".touch-actions", b"orientation: landscape", b"min-height: 2.75rem")
    if not all(marker in page for marker in required_page):
        raise RuntimeError("browser page lacks the complete touch control overlay")
    if not all(marker in script for marker in required_script):
        raise RuntimeError("browser client lacks touch control detection or shared input routing")
    if not all(marker in styles for marker in required_styles):
        raise RuntimeError("browser styles lack responsive touch control layout")


def malformed_client(host: str, port: int) -> None:
    client = WebSocketClient(host, port)
    client.send_frame(1, b"not json")
    deadline = time.monotonic() + 4
    saw_error = False
    while time.monotonic() < deadline:
        opcode, payload = client.receive_frame()
        if opcode == 1 and json.loads(payload).get("type") == "error":
            saw_error = True
        if opcode == 8:
            break
    if not saw_error:
        raise RuntimeError("malformed browser input did not produce a protocol error")
    client.close_abruptly()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--item-flow", action="store_true")
    args = parser.parse_args()
    base_url = f"http://{args.host}:{args.port}/"
    fetch_page(base_url)
    verify_touch_ui(base_url)

    if args.item_flow:
        client = WebSocketClient(args.host, args.port)
        if client.seed != 14:
            raise RuntimeError("browser item-flow seed was not applied")
        for move in "ccccdd":
            client.input(move)
            client.drain_output()
        client.reset_capture()
        client.input("g")
        client.wait_for_screen("HealthPotion")
        client.input("u1")
        client.wait_for_screen("drink the Health Potion")
        client.send_json({"type": "quit"})
        client.wait_for_close()
        print("Browser item flow verification passed")
        return 0

    moving = WebSocketClient(args.host, args.port, fragmented_hello=True)
    helping = WebSocketClient(args.host, args.port)
    if moving.seed != 29 or helping.seed != 29:
        raise RuntimeError("browser deterministic test seed was not applied")

    moving.input("2")
    moving.wait_for_screen("TARGETING")
    moving.input("\x1b")
    moving.input("[C")
    moving.wait_for_screen("TARGETING")
    moving.input("\x1b[")
    moving.input("Z")
    moving.input("\r")
    moving.wait_for_screen("Grave Bolt strikes")
    moving.wait_for_screen("T1  Seed")
    if "T1  Seed" in helping.screen_text():
        raise RuntimeError("targeting leaked across browser sessions")

    moving.input("s")
    moving.resize(81, 24)
    moving.wait_for_screen("T2  Seed")
    moving.input("w")
    moving.reset_capture()
    moving.input("1")
    moving.resize(82, 24)
    moving.wait_for_screen("1 Cleave")
    moving.reset_capture()
    moving.input("r")
    moving.resize(83, 24)
    moving.wait_for_screen("The dungeon reforms around you.")

    moving.reset_capture()
    moving.input("u")
    moving.wait_for_screen("USE ITEM")
    if "USE ITEM" in helping.screen_text():
        raise RuntimeError("item-use mode leaked across browser sessions")
    moving.input("\x1b")
    moving.resize(84, 24)
    moving.wait_for_screen("GRAVE KNIGHT")

    moving.reset_capture()
    moving.input("i")
    moving.wait_for_screen("INSPECT")
    moving.input("\x1b")
    moving.input("[C")
    moving.resize(84, 24)
    moving.wait_for_screen("INSPECT")
    if "INSPECT" in helping.screen_text():
        raise RuntimeError("inspection leaked across browser sessions")
    moving.reset_capture()
    moving.input("\x1b")
    moving.wait_for_screen("GRAVE KNIGHT")

    helping.input("?")
    helping.resize(81, 24)
    helping.wait_for_screen("?: close | r: restart | Q: quit")
    helping.wait_for_screen("T0  Seed")

    moving.resize(39, 20)
    moving.wait_for_screen("Terminal too small: 39x20")
    moving.resize(80, 24)
    moving.wait_for_screen("GRAVE KNIGHT")

    malformed_client(args.host, args.port)

    slow = WebSocketClient(args.host, args.port)
    for _ in range(200):
        slow.input("s")
    healthy = WebSocketClient(args.host, args.port)
    healthy.input("?")
    healthy.resize(83, 24)
    healthy.wait_for_screen("g picks up; u then 1-4 uses an item.")
    slow.close_abruptly()

    moving.input("Q")
    moving.wait_for_close()
    helping.send_json({"type": "quit"})
    helping.wait_for_close()
    healthy.send_json({"type": "quit"})
    healthy.wait_for_close()

    reconnect = WebSocketClient(args.host, args.port)
    reconnect.input("2")
    reconnect.wait_for_screen("TARGETING")
    reconnect.reset_capture()
    reconnect.input("\x1b")
    reconnect.wait_for_screen("GRAVE KNIGHT")
    reconnect.send_json({"type": "quit"})
    reconnect.wait_for_close()
    fetch_page(base_url)
    print("Browser transport verification passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
