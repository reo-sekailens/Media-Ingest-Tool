/* global fetch, AbortSignal */
import { writeFile } from "node:fs/promises";
import { setTimeout } from "node:timers/promises";
import { Buffer } from "node:buffer";

const elementKey = "element-6066-11e4-a52e-4f735466cecf";

// This adapter only speaks the embedded driver's W3C HTTP protocol. It does not
// install Tauri API mocks, alter command responses, or use a browser dev server.
export class NativeWebDriver {
  constructor(port, request = fetch) {
    if (!Number.isInteger(port) || port < 1 || port > 65535) {
      throw new Error("A valid loopback WebDriver port is required");
    }
    this.baseUrl = `http://127.0.0.1:${port}`;
    this.request = request;
    this.sessionId = null;
  }

  async command(method, endpoint, body) {
    const response = await this.request(`${this.baseUrl}${endpoint}`, {
      method,
      headers: { "Content-Type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: AbortSignal.timeout(30_000),
    });
    const data = await response.json();
    if (!response.ok || data.value?.error) {
      // Native exception messages can contain private paths or identifiers.
      // Keep only the standardized error category in published test output.
      const category = String(data.value?.error || "unknown error").replace(
        /[^a-z -]/gi,
        "",
      );
      throw new Error(
        `WebDriver ${method} failed: HTTP ${response.status}, ${category}`,
      );
    }
    if (!Object.hasOwn(data, "value")) {
      throw new Error("WebDriver response is missing its W3C value envelope");
    }
    return data.value;
  }

  async createSession() {
    const value = await this.command("POST", "/session", {
      capabilities: {
        alwaysMatch: { timeouts: { script: 30_000, pageLoad: 30_000, implicit: 0 } },
      },
    });
    if (typeof value?.sessionId !== "string" || !value.sessionId) {
      throw new Error("Embedded WebDriver did not create a session");
    }
    this.sessionId = value.sessionId;
    return value.capabilities;
  }

  sessionEndpoint(endpoint) {
    if (!this.sessionId) throw new Error("A WebDriver session must be created first");
    return `/session/${encodeURIComponent(this.sessionId)}${endpoint}`;
  }

  async execute(fn, ...args) {
    return await this.command("POST", this.sessionEndpoint("/execute/sync"), {
      script: `return (${fn.toString()}).apply(null, arguments);`,
      args,
    });
  }

  async click(selector) {
    const element = await this.command("POST", this.sessionEndpoint("/element"), {
      using: "css selector",
      value: selector,
    });
    if (typeof element?.[elementKey] !== "string") {
      throw new Error("WebDriver did not return an element reference");
    }
    await this.command(
      "POST",
      this.sessionEndpoint(`/element/${encodeURIComponent(element[elementKey])}/click`),
      {},
    );
  }

  async saveScreenshot(filePath) {
    const base64 = await this.command("GET", this.sessionEndpoint("/screenshot"));
    if (typeof base64 !== "string") {
      throw new Error("Native screenshot response must be a base64 PNG");
    }
    const png = Buffer.from(base64, "base64");
    await writeFile(filePath, png, { flag: "wx" });
    return png;
  }

  async deleteSession() {
    if (!this.sessionId) return;
    await this.command("DELETE", this.sessionEndpoint(""));
    this.sessionId = null;
  }

  async waitUntil(
    check,
    { timeout = 30_000, timeoutMsg = "Native UI did not settle" } = {},
  ) {
    const deadline = Date.now() + timeout;
    do {
      if (await check()) return;
      await setTimeout(100);
    } while (Date.now() < deadline);
    throw new Error(timeoutMsg);
  }
}
