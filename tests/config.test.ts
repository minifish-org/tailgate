import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { loadConfig } from "../src/config.js";

const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "tailgate-config-"));
const configPath = path.join(tempDir, "config.yaml");

fs.writeFileSync(
  configPath,
  `
server:
  host: 127.0.0.1
  port: 11435

local:
  base_url: http://qwen-local.test/v1
  api_key_env: LOCAL_API_KEY
  max_concurrency: 1
`,
);

try {
  const config = loadConfig(configPath);

  assert.equal(config.models["local/asr"]?.upstream_model, "local-asr");
  assert.equal(config.models["local/asr"]?.endpoint, "audio_transcriptions");
  assert.equal(config.models["local/translation"]?.upstream_model, "local-translation");
  assert.equal(config.models["local/translation"]?.endpoint, "translations");

  assert.equal(
    config.models["local/tts-voice-design"]?.upstream_model,
    "local-tts-voice-design",
  );
  assert.equal(config.models["local/tts-voice-design"]?.endpoint, "audio_speech");
  assert.equal(config.models["local/tts-voice-design"]?.max_concurrency, 1);
} finally {
  fs.rmSync(tempDir, { recursive: true, force: true });
}

console.log("config tests passed");
