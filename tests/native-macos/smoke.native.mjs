/* global window, document, navigator, process */
// Explicitly invoked on macOS; this is excluded from Vitest's filename pattern.
import assert from "node:assert/strict";
import { writeFile } from "node:fs/promises";
import path from "node:path";
import { describe, it, before, after } from "node:test";
import { NativeWebDriver } from "./webdriver.mjs";

const outputDirectory = process.env.MEDIA_INGEST_MACOS_UI_OUTPUT;
const browser = new NativeWebDriver(Number(process.env.TAURI_WEBDRIVER_PORT));
const evidence = {
  evidenceKind: "native-wkwebview-test-build",
  testDriver: "tauri-plugin-wdio-webdriver",
  mockedCommands: false,
  startupComplete: false,
  tests: {},
  limitations: [
    "The embedded driver synthesizes DOM actions; physical keyboard behavior is not certified.",
    "No ingest, eject, registration, formatting, permission-dialog, or hardware operation is tested.",
    "The instrumented app is separate from shipping packages and Gatekeeper installation checks.",
  ],
};
let snapshot;

function startupStage(stage) {
  evidence.startupStage = stage;
  process.stdout.write(`Native startup stage: ${stage}\n`);
}

async function invokeReadOnly(command) {
  assert.ok(
    ["get_device_snapshot", "get_ingest_history"].includes(command),
    "Only read-only startup commands are permitted in this smoke suite",
  );
  const response = await browser.execute(async (name) => {
    return await window.__TAURI__.core.invoke(name);
  }, command);
  assert.equal(response?.contractVersion, 1, "Native IPC contract must be version 1");
  assert.ok(response?.error === null, "Native startup command must succeed");
  assert.ok(
    response && Object.hasOwn(response, "data") && response.data !== null,
    "Native startup command must return data",
  );
  return response.data;
}

async function renderedInventoryMatchesNative() {
  return await browser.execute((deviceCount) => {
    const fixtureLeak = ["A-Cam • SDXC", "B-Cam • microSDXC"].some((label) =>
      document.body.innerText.includes(label),
    );
    if (fixtureLeak) return false;
    if (deviceCount === 0) {
      return (
        document.body.innerText.includes("No removable media is mounted.") &&
        [...document.querySelectorAll("button")].some(
          (button) => button.textContent.trim() === "Refresh Inventory",
        ) &&
        document.querySelectorAll('[aria-label="Connected media"] > li').length === 0
      );
    }
    return (
      document.querySelectorAll('[aria-label="Connected media"] > li').length ===
        deviceCount &&
      document.querySelectorAll(
        '[aria-label="Connected media"] button[aria-pressed="true"]',
      ).length === 1
    );
  }, snapshot.devices.length);
}

async function focusButton(selector) {
  const focused = await browser.execute((buttonSelector) => {
    const button = document.querySelector(buttonSelector);
    if (!(button instanceof window.HTMLButtonElement) || button.disabled) return false;
    button.focus();
    return document.activeElement === button && button.type === "button";
  }, selector);
  assert.ok(focused, "Visible action must be an enabled focusable HTML button");
}

describe("macOS native startup and read-only workspace", () => {
  before(async () => {
    assert.equal(process.platform, "darwin", "This suite requires actual macOS");
    startupStage("create-session");
    await browser.createSession();
    startupStage("wait-frontend");
    await browser.waitUntil(
      async () =>
        await browser.execute(
          () =>
            document.readyState === "complete" &&
            typeof window.__TAURI__?.core?.invoke === "function" &&
            Boolean(document.querySelector("main")),
        ),
      { timeoutMsg: "The packaged WKWebView and real Tauri IPC did not initialize" },
    );
    startupStage("read-native-inventory");
    snapshot = await invokeReadOnly("get_device_snapshot");
    assert.ok(Array.isArray(snapshot.devices), "Native inventory must be an array");
    assert.ok(Number.isInteger(snapshot.sequence), "Native snapshot must be sequenced");
    assert.ok(snapshot.sequence > 0, "The inventory must come from native discovery");
    evidence.inventoryCount = snapshot.devices.length;
    evidence.inventorySummary = snapshot.devices.map((device) => ({
      state: device.state,
      filesystem: device.details.filesystem,
      confidence: device.identity.confidence,
      mountCount: device.details.mountLocations.length,
    }));
    // Publish only classifications/counts; never host paths, names or identifiers.
    process.stdout.write(
      `Native inventory summary: ${JSON.stringify(evidence.inventorySummary)}\n`,
    );
    startupStage("wait-rendered-inventory");
    await browser.waitUntil(renderedInventoryMatchesNative, {
      timeoutMsg: "The UI did not replace preview fixtures with native inventory",
    });
    evidence.startupComplete = true;
  });

  it("loads the bundled frontend and production stylesheet in WKWebView", async () => {
    const renderer = await browser.execute(async () => {
      const main = document.querySelector("main");
      const styles = window.getComputedStyle(main);
      return {
        nativeBridge: typeof window.__TAURI_INTERNALS__?.invoke === "function",
        identifier: await window.__TAURI__.app.getIdentifier(),
        webkit: /AppleWebKit/.test(navigator.userAgent),
        chromium: /Chrome|Chromium/.test(navigator.userAgent),
        protocol: window.location.protocol,
        stylesheetCount: document.styleSheets.length,
        backgroundColor: styles.backgroundColor,
        backgroundImage: styles.backgroundImage,
        width: main.getBoundingClientRect().width,
        fixtureMocks: Boolean(window.__wdio_mocks__),
        overflow:
          document.documentElement.scrollWidth >
          document.documentElement.clientWidth + 1,
      };
    });
    assert.ok(renderer.nativeBridge && renderer.webkit && !renderer.chromium);
    assert.equal(renderer.identifier, "com.mediaingest.tool.macos-ui-tests");
    assert.equal(renderer.protocol, "tauri:", "Bundled app must use its native origin");
    assert.ok(renderer.stylesheetCount > 0, "Bundled CSS must load under the app CSP");
    assert.ok(
      renderer.backgroundColor !== "rgba(0, 0, 0, 0)" ||
        renderer.backgroundImage !== "none",
      "The bundled stylesheet must render a color or gradient background",
    );
    assert.ok(renderer.width >= 960, "The declared desktop minimum width must apply");
    assert.equal(renderer.fixtureMocks, false, "No command mocks may be installed");
    assert.equal(renderer.overflow, false, "Native workspace must not overflow");
    evidence.tests.bundledRenderer = "passed";
  });

  it("reads real local history through the native IPC contract", async () => {
    const history = await invokeReadOnly("get_ingest_history");
    assert.ok(Array.isArray(history), "Native history must be an array");
    assert.equal(
      history.length,
      0,
      "The isolated test identity must have fresh history",
    );
    evidence.historyCount = history.length;
    evidence.tests.nativeHistory = "passed";
  });

  it("refreshes the observed inventory without a source operation", async () => {
    const selector =
      snapshot.devices.length > 0 ? ".source-rail > button" : "main button";
    await focusButton(selector);
    await browser.click(selector);
    snapshot = await invokeReadOnly("get_device_snapshot");
    await browser.waitUntil(renderedInventoryMatchesNative);
    evidence.tests.nativeInventoryRefresh = "passed";
  });

  it("shows the empty state only when native inventory is empty", async (context) => {
    if (snapshot.devices.length > 0) {
      evidence.tests.emptyState = "skipped: native inventory is nonempty";
      context.skip("Native media observed");
      return;
    }
    await browser.waitUntil(renderedInventoryMatchesNative);
    evidence.tests.emptyState = "passed";
  });

  it("checks theme and identity panels only when native media exists", async (context) => {
    if (snapshot.devices.length === 0) {
      evidence.tests.mediaPanels = "skipped: no native media; empty UI hides panels";
      context.skip("No native media; empty UI hides these panels");
      return;
    }
    const originalDark = await browser.execute(() =>
      document.documentElement.classList.contains("dark"),
    );
    const themeSelector = 'button[aria-label^="Switch to "]';
    await focusButton(themeSelector);
    await browser.click(themeSelector);
    await browser.waitUntil(
      async () =>
        (await browser.execute(() =>
          document.documentElement.classList.contains("dark"),
        )) !== originalDark,
    );
    await browser.click(themeSelector);
    await browser.waitUntil(
      async () =>
        (await browser.execute(() =>
          document.documentElement.classList.contains("dark"),
        )) === originalDark,
    );
    const detailsSelector = 'button[aria-controls="identity-details"]';
    await focusButton(detailsSelector);
    const expanded = async () =>
      await browser.execute(
        (selector) => document.querySelector(selector)?.getAttribute("aria-expanded"),
        detailsSelector,
      );
    assert.equal(await expanded(), "false");
    await browser.click(detailsSelector);
    await browser.waitUntil(
      async () =>
        await browser.execute(() =>
          Boolean(document.querySelector("#identity-details")),
        ),
    );
    assert.equal(await expanded(), "true");
    await browser.click(detailsSelector);
    assert.equal(await expanded(), "false");
    evidence.tests.mediaPanels = "passed: focusability and driver click only";
  });

  it("saves a native screenshot with sensitive text hidden in place", async () => {
    const privateValues = snapshot.devices
      .flatMap((device) => [
        device.details.displayName,
        ...device.details.mountLocations,
        device.details.readerFingerprint,
        device.details.readerSlot,
        device.identity.mediaKey,
        ...device.identity.evidence.map((entry) => entry.fingerprint),
      ])
      .filter((value) => typeof value === "string" && value.length > 0);
    const redactionCount = await browser.execute((values) => {
      if (document.activeElement instanceof window.HTMLElement) {
        document.activeElement.blur();
      }
      window.scrollTo(0, 0);
      const targets = new Set(
        document.querySelectorAll(
          '#device-title, [aria-label="Connected media"] li strong, .workspace-content dd, [aria-label="Saved configuration"] p, [aria-label="Saved configuration"] span, [aria-label="Saved configuration"] strong, .workspace-content input, .workspace-content textarea',
        ),
      );
      const walker = document.createTreeWalker(
        document.body,
        window.NodeFilter.SHOW_TEXT,
      );
      while (walker.nextNode()) {
        const node = walker.currentNode;
        if (
          values.some((value) => node.textContent.includes(value)) &&
          node.parentElement
        ) {
          targets.add(node.parentElement);
        }
      }
      for (const element of targets)
        element.setAttribute("data-native-smoke-redacted", "");
      const style = document.createElement("style");
      style.id = "native-smoke-redaction-style";
      // Visibility preserves actual sizes and positions while completely
      // excluding sensitive text from pixels. It is removed after capture.
      style.textContent =
        "[data-native-smoke-redacted] { visibility: hidden !important; }";
      document.head.append(style);
      return targets.size;
    }, privateValues);
    evidence.screenshotRedactions = {
      applied: redactionCount > 0,
      elementCount: redactionCount,
      scope:
        "device names, mount paths, opaque identities, reader references, destination/configuration values",
      layoutPreserved: true,
    };
    const filename =
      snapshot.devices.length > 0
        ? "native-workspace-redacted.png"
        : "native-empty-state.png";
    let png;
    try {
      png = await browser.saveScreenshot(path.join(outputDirectory, filename));
    } finally {
      await browser.execute(() => {
        for (const element of document.querySelectorAll(
          "[data-native-smoke-redacted]",
        )) {
          element.removeAttribute("data-native-smoke-redacted");
        }
        document.querySelector("#native-smoke-redaction-style")?.remove();
      });
    }
    assert.equal(png.subarray(0, 8).toString("hex"), "89504e470d0a1a0a");
    assert.ok(png.readUInt32BE(16) >= 960, "Native screenshot must have desktop width");
    assert.ok(
      png.readUInt32BE(20) >= 640,
      "Native screenshot must have desktop height",
    );
    evidence.tests.nativeScreenshot = "passed";
    evidence.screenshotFilename = filename;
  });

  after(async () => {
    try {
      await writeFile(
        path.join(outputDirectory, "smoke-summary.json"),
        `${JSON.stringify(evidence, null, 2)}\n`,
        { encoding: "utf8", flag: "wx" },
      );
    } finally {
      await browser.deleteSession();
    }
  });
});
