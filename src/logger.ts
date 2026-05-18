import pino from "pino";

export const logger = pino({
  level: process.env.LOG_LEVEL || "info",
  redact: {
    paths: [
      "req.headers.authorization",
      "headers.authorization",
      "*.api_key",
      "*.apiKey",
      "*.provider_api_key",
    ],
    remove: true,
  },
});
