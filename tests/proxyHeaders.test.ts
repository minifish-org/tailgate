import assert from "node:assert/strict";
import { filteredUpstreamHeaders } from "../src/openaiProxy.js";

const headers = new Headers({
  "content-type": "application/json",
  "content-encoding": "br",
  "content-length": "123",
  connection: "keep-alive",
  "x-provider": "ok",
});

const filtered = filteredUpstreamHeaders(headers);

assert.equal(filtered.get("content-type"), "application/json");
assert.equal(filtered.get("x-provider"), "ok");
assert.equal(filtered.has("content-encoding"), false);
assert.equal(filtered.has("content-length"), false);
assert.equal(filtered.has("connection"), false);

console.log("proxy header tests passed");
