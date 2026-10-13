import assert from "node:assert/strict";
import { test } from "node:test";
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

test("creates, uses and deletes an actual W3C session envelope", async () => {
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
  assert.equal(
    requests[1].url,
    "http://127.0.0.1:4445/session/native%2Fsession/execute/sync",
  );
  const execute = JSON.parse(requests[1].body);
  assert.deepEqual(execute.args, [2]);
  assert.match(execute.script, /^return \(/);
  await driver.deleteSession();
  assert.equal(requests[2].method, "DELETE");
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
