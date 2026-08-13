import { Terminal } from "/vendor/xterm-6.0.0.mjs";
import { FitAddon } from "/vendor/addon-fit-0.11.0.mjs";

const statusElement = document.querySelector("#status");
const reconnectButton = document.querySelector("#reconnect");
const terminalElement = document.querySelector("#terminal");
const touchControls = document.querySelector("#touch-controls");
const touchToggle = document.querySelector("#touch-toggle");
const touchHide = document.querySelector("#touch-hide");
const actionsToggle = document.querySelector("#actions-toggle");
const actionsMenu = document.querySelector("#actions-menu");
const nextTargetButton = document.querySelector("#next-target");

const terminal = new Terminal({
  allowProposedApi: false,
  convertEol: false,
  cursorBlink: false,
  disableStdin: true,
  fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
  fontSize: 15,
  scrollback: 0,
  theme: {
    background: "#090a0c",
    foreground: "#d7d0bd",
    cursor: "#d6b75d",
    selectionBackground: "#5b4c2f88"
  }
});
const fitAddon = new FitAddon();
terminal.loadAddon(fitAddon);
terminal.open(terminalElement);
terminal.attachCustomKeyEventHandler((event) => !(event.ctrlKey || event.metaKey || event.altKey));

let socket = null;
let ready = false;
let fitting = false;
let targeting = false;

function send(message) {
  if (socket?.readyState === WebSocket.OPEN) {
    socket.send(JSON.stringify({ v: 1, ...message }));
  }
}

function sendInput(data, focus = true) {
  if (ready) send({ type: "input", data });
  if (focus) terminal.focus();
}

function sendTouchInput(value) {
  sendInput(touchInput(value), !isTouchCapable());
}

function touchInput(value) {
  return { tab: "\t", enter: "\r", escape: "\x1b" }[value] ?? value;
}

function setTouchControlsVisible(visible) {
  touchControls.hidden = !visible;
  touchToggle.setAttribute("aria-expanded", String(visible));
  touchToggle.textContent = visible ? "Hide controls" : "Controls";
}

function isTouchCapable() {
  return navigator.maxTouchPoints > 0 || matchMedia("(pointer: coarse)").matches;
}

function setTargeting(active) {
  targeting = active;
  nextTargetButton.disabled = !active;
  nextTargetButton.setAttribute("aria-disabled", String(!active));
}

function setActionsMenuVisible(visible) {
  actionsMenu.hidden = !visible;
  actionsToggle.setAttribute("aria-expanded", String(visible));
}

function fit() {
  if (fitting) return;
  fitting = true;
  requestAnimationFrame(() => {
    fitting = false;
    fitAddon.fit();
  });
}

function showDisconnected(message) {
  ready = false;
  terminal.options.disableStdin = true;
  statusElement.textContent = message;
  reconnectButton.hidden = false;
}

function connect() {
  reconnectButton.hidden = true;
  statusElement.textContent = "Connecting…";
  terminal.reset();
  fitAddon.fit();
  setTargeting(false);
  const scheme = location.protocol === "https:" ? "wss" : "ws";
  socket = new WebSocket(`${scheme}://${location.host}/ws`);
  socket.binaryType = "arraybuffer";
  socket.addEventListener("open", () => {
    send({ type: "hello", cols: terminal.cols, rows: terminal.rows });
  });
  socket.addEventListener("message", (event) => {
    if (typeof event.data !== "string") {
      terminal.write(new Uint8Array(event.data));
      return;
    }
    const message = JSON.parse(event.data);
    if (message.v !== 1) {
      socket.close(1002, "unsupported protocol");
    } else if (message.type === "ready") {
      ready = true;
      terminal.options.disableStdin = false;
      statusElement.textContent = `Connected · seed ${message.seed}`;
      terminal.focus();
    } else if (message.type === "state") {
      setTargeting(Boolean(message.targeting));
    } else if (message.type === "error") {
      showDisconnected(message.message || "Connection error");
    }
  });
  socket.addEventListener("error", () => showDisconnected("Connection failed"));
  socket.addEventListener("close", (event) => {
    showDisconnected(event.wasClean ? "Disconnected" : "Connection lost");
  });
}

terminal.onData((data) => {
  sendInput(data);
});
terminal.onResize(({ cols, rows }) => {
  if (ready) send({ type: "resize", cols, rows });
});
terminalElement.addEventListener("click", () => terminal.focus());
reconnectButton.addEventListener("click", connect);
touchToggle.addEventListener("click", () => setTouchControlsVisible(touchControls.hidden));
touchHide.addEventListener("click", () => setTouchControlsVisible(false));
actionsToggle.addEventListener("click", () => setActionsMenuVisible(actionsMenu.hidden));
touchControls.addEventListener("click", (event) => {
  const button = event.target.closest("button[data-input]");
  if (button && !(button.dataset.input === "tab" && !targeting)) {
    sendTouchInput(button.dataset.input);
  }
});
matchMedia("(pointer: coarse)").addEventListener("change", (event) => {
  if (event.matches) setTouchControlsVisible(true);
});
if (isTouchCapable()) setTouchControlsVisible(true);
new ResizeObserver(fit).observe(document.querySelector("#terminal-shell"));

connect();
