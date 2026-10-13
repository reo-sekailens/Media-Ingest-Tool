/* global fetch, AbortSignal */
import { writeFile } from "node:fs/promises";
import { setTimeout } from "node:timers/promises";
import { Buffer } from "node:buffer";

const elementKey = "element-6066-11e4-a52e-4f735466cecf";
const standardErrors = new Set([
  "invalid argument",
  "invalid session id",
  "javascript error",
  "no such element",
  "no such window",
  "script timeout",
  "session not created",
  "stale element reference",
  "timeout",
  "unknown command",
  "unknown error",
  "unsupported operation",
]);

function endpointCategory(endpoint) {
  if (endpoint === "/session") return "create-session";
  if (endpoint === "/status") return "status";
  if (endpoint === "/wdio/eval") return "direct-eval";
  const routes = [
    [/^\/session\/[^/]+\/execute\/sync$/, "execute-sync"],
    [/^\/session\/[^/]+\/element$/, "find-element"],
    [/^\/session\/[^/]+\/element\/[^/]+\/click$/, "element-click"],
    [/^\/session\/[^/]+\/screenshot$/, "screenshot"],
    [/^\/session\/[^/]+$/, "session"],
  ];
  return routes.find(([pattern]) => pattern.test(endpoint))?.[1] || "other";
}

// This adapter speaks W3C commands and the official embedded DirectEval route.
// It does not install Tauri API mocks, alter command responses, or use a dev server.
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
    const label = `WebDriver ${method} ${endpointCategory(endpoint)}`;
    let response;
    try {
      response = await this.request(`${this.baseUrl}${endpoint}`, {
        method,
        headers: { "Content-Type": "application/json" },
        body: body === undefined ? undefined : JSON.stringify(body),
        signal: AbortSignal.timeout(30_000),
      });
    } catch (error) {
      const category =
        error?.name === "TimeoutError"
          ? "request timeout"
          : error?.name === "AbortError"
            ? "request aborted"
            : "request rejected";
      // Raw transport causes can expose private URLs, scripts or native paths.
      // eslint-disable-next-line preserve-caught-error
      throw new Error(`${label} failed: ${category}`);
    }
    let data;
    try {
      data = await response.json();
    } catch {
      throw new Error(`${label} failed: response decoding failed`);
    }
    const directEval = endpoint === "/wdio/eval";
    if (
      !response.ok ||
      (!directEval && data?.value?.error) ||
      (directEval && data?.error)
    ) {
      // Native exception messages can contain private paths or identifiers.
      // Keep only the standardized error category in published test output.
      const category = directEval
        ? "script evaluation failed"
        : standardErrors.has(data?.value?.error)
          ? data.value.error
          : "unknown error";
      throw new Error(`${label} failed: HTTP ${response.status}, ${category}`);
    }
    if (directEval && data?.undef === true) return undefined;
    if (!data || !Object.hasOwn(data, "value")) {
      throw new Error(
        `${label} response is missing its ${directEval ? "direct-eval" : "W3C"} value envelope`,
      );
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
    if (!this.sessionId) throw new Error("A WebDriver session must be created first");
    // The pinned official Tauri service uses /wdio/eval for the embedded
    // provider. Its callback contract reaches the macOS callAsyncJavaScript
    // result channel; W3C execute/sync uses evaluateJavaScript polling instead.
    return await this.command("POST", "/wdio/eval", {
      script: `var __cb = arguments[arguments.length - 1];
(async () => {
  try {
    const __result = await (${fn.toString()}).apply(null, ${JSON.stringify(args)});
    __cb({ ok: true, value: __result === undefined ? null : __result, undef: __result === undefined });
  } catch {
    __cb({ ok: false, error: "Native evaluation failed" });
  }
})();`,
      window_label: "main",
      timeout_ms: 30_000,
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
