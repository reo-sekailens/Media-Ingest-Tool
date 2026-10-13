import assert from "node:assert/strict";
import { test } from "node:test";
import { runInNewContext } from "node:vm";
import { NativeWebDriver } from "./webdriver.mjs";

// Transport-contract fixtures check the small HTTP adapter. They do not stand
// in for native IPC, rendering, or device evidence from smoke.native.mjs.
function fixtureDriver(responses) {
  const requests = [];
  const driver = new NativeWebDriver(4445, async (url, options) => {
    requests.push({ url, ...options });
    const response = responses.shift();
    assert.ok(response, "Unexpected WebDriver request");
    return {
      ok: response.status === undefined || response.status === 200,
      status: response.status || 200,
      json: async () => response.body,
    };
  });
  return { driver, requests };
}

test("restricts the driver address to a valid loopback port", () => {
  for (const port of [0, 65536, NaN, "4445", 4.5]) {
    assert.throws(() => new NativeWebDriver(port), /valid loopback/);
  }
  assert.equal(new NativeWebDriver(4445).baseUrl, "http://127.0.0.1:4445");
});

test("creates/deletes W3C sessions and evaluates via the official embedded route", async () => {
  const { driver, requests } = fixtureDriver([
    {
      body: {
        value: { sessionId: "native/session", capabilities: { platformName: "macos" } },
      },
    },
    { body: { value: 3 } },
    { body: { value: null } },
  ]);
  assert.equal((await driver.createSession()).platformName, "macos");
  assert.equal(await driver.execute((number) => number + 1, 2), 3);
  assert.equal(requests[1].url, "http://127.0.0.1:4445/wdio/eval");
  const execute = JSON.parse(requests[1].body);
  assert.equal(execute.window_label, "main");
  assert.equal(execute.timeout_ms, 30_000);
  const callback = await new Promise((resolve) => {
    runInNewContext(`(function() { ${execute.script} }).call(null, callback);`, {
      callback: resolve,
    });
  });
  assert.equal(callback.ok, true);
  assert.equal(callback.value, 3);
  assert.equal(callback.undef, false);
  await driver.deleteSession();
  assert.equal(requests[2].method, "DELETE");
  assert.equal(requests[2].url, "http://127.0.0.1:4445/session/native%2Fsession");
  assert.equal(driver.sessionId, null);
});

test("rejects session use before initialization and malformed session envelopes", async () => {
  assert.throws(
    () => new NativeWebDriver(4445).sessionEndpoint("/screenshot"),
    /created first/,
  );
  const { driver } = fixtureDriver([{ body: { value: { sessionId: null } } }]);
  await assert.rejects(driver.createSession(), /did not create a session/);
});

test("finds elements with CSS and clicks the returned W3C element reference", async () => {
  const { driver, requests } = fixtureDriver([
    { body: { value: { "element-6066-11e4-a52e-4f735466cecf": "control/1" } } },
    { body: { value: null } },
  ]);
  driver.sessionId = "native";
  await driver.click("main button");
  assert.deepEqual(JSON.parse(requests[0].body), {
    using: "css selector",
    value: "main button",
  });
  assert.equal(
    requests[1].url,
    "http://127.0.0.1:4445/session/native/element/control%2F1/click",
  );
});

test("keeps native paths and identifiers out of standard WebDriver failures", async () => {
  const { driver } = fixtureDriver([
    {
      status: 500,
      body: {
        value: { error: "javascript error", message: "/Users/private/secret-card" },
      },
    },
  ]);
  await assert.rejects(driver.command("GET", "/status"), (error) => {
    assert.match(error.message, /HTTP 500, javascript error/);
    assert.doesNotMatch(error.message, /private|secret|Users/);
    return true;
  });
});

test("labels rejected and timed-out endpoints without leaking transport data", async () => {
  for (const [name, category] of [
    ["TypeError", "request rejected"],
    ["TimeoutError", "request timeout"],
    ["AbortError", "request aborted"],
  ]) {
    const privateText =
      "/Users/private/secret-card native-session-private script-private";
    const driver = new NativeWebDriver(4445, async () => {
      const error = new Error(privateText);
      error.name = name;
      throw error;
    });
    driver.sessionId = "native-session-private";
    await assert.rejects(
      driver.execute(() => "script-private"),
      (error) => {
        assert.equal(error.message, `WebDriver POST direct-eval failed: ${category}`);
        assert.doesNotMatch(
          error.stack,
          /\/Users\/private|secret-card|native-session-private|script-private|127\.0\.0\.1/,
        );
        assert.equal(error.cause, undefined);
        return true;
      },
    );
    await assert.rejects(
      driver.createSession(),
      new RegExp(`WebDriver POST create-session failed: ${category}`),
    );
  }
});

test("awaits DirectEval promises and reports undefined and script failures safely", async () => {
  const { driver, requests } = fixtureDriver([
    { body: { value: { count: 2 } } },
    { body: { value: null, undef: true } },
    { status: 500, body: { error: "/Users/private/secret-card" } },
  ]);
  driver.sessionId = "native";
  assert.deepEqual(await driver.execute(async (count) => ({ count }), 2), { count: 2 });
  assert.equal(await driver.execute(() => undefined), undefined);
  await assert.rejects(
    driver.execute(() => {
      throw new Error("private-script-error");
    }),
    (error) => {
      assert.equal(
        error.message,
        "WebDriver POST direct-eval failed: HTTP 500, script evaluation failed",
      );
      assert.doesNotMatch(error.stack, /private-script-error|secret-card/);
      return true;
    },
  );
  for (const [index, expected] of [
    [0, { ok: true, value: { count: 2 }, undef: false }],
    [1, { ok: true, value: null, undef: true }],
    [2, { ok: false, error: "Native evaluation failed" }],
  ]) {
    const result = await new Promise((resolve) => {
      const { script } = JSON.parse(requests[index].body);
      runInNewContext(`(function() { ${script} }).call(null, callback);`, {
        callback: resolve,
      });
    });
    assert.deepEqual(JSON.parse(JSON.stringify(result)), expected);
  }
});

test("sanitizes response decoding and unrecognized server error categories", async () => {
  const decoder = new NativeWebDriver(4445, async () => ({
    json: async () => {
      throw new Error("private response with /Users/secret-card");
    },
  }));
  await assert.rejects(decoder.command("GET", "/status"), (error) => {
    assert.equal(
      error.message,
      "WebDriver GET status failed: response decoding failed",
    );
    assert.doesNotMatch(error.stack, /private response|secret-card/);
    return true;
  });
  const { driver } = fixtureDriver([
    { status: 500, body: { value: { error: "private secret card" } } },
  ]);
  await assert.rejects(driver.command("GET", "/status"), (error) => {
    assert.equal(error.message, "WebDriver GET status failed: HTTP 500, unknown error");
    assert.doesNotMatch(error.stack, /private|secret/);
    return true;
  });
});

test("rejects success responses without the W3C value envelope", async () => {
  const { driver } = fixtureDriver([{ body: { ready: true } }]);
  await assert.rejects(
    driver.command("GET", "/status"),
    /missing its W3C value envelope/,
  );
});

test("bounds a UI wait and reports its concrete failure", async () => {
  const driver = new NativeWebDriver(4445);
  await driver.waitUntil(async () => true, { timeout: 1 });
  await assert.rejects(
    driver.waitUntil(async () => false, {
      timeout: 1,
      timeoutMsg: "rendering never settled",
    }),
    /rendering never settled/,
  );
});
