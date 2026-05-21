import { Context, Next } from "hono";

const ALLOW_METHODS = "POST, OPTIONS";
const ALLOW_HEADERS = "authorization, content-type";
const ALLOW_PRIVATE_NETWORK = "true";

export function openAICors(allowedOrigins: string[]) {
  const allowed = new Set(allowedOrigins);
  const allowAnyOrigin = allowed.has("*");

  return async (c: Context, next: Next) => {
    const origin = c.req.header("origin");
    const corsOrigin = origin && (allowAnyOrigin || allowed.has(origin)) ? (allowAnyOrigin ? "*" : origin) : undefined;

    if (c.req.method === "OPTIONS") {
      applyCorsHeaders(c, corsOrigin);
      return c.body(null, 204);
    }

    await next();
    applyCorsHeaders(c, corsOrigin);
  };
}

function applyCorsHeaders(c: Context, origin: string | undefined) {
  if (!origin) return;
  c.res.headers.set("Access-Control-Allow-Origin", origin);
  c.res.headers.set("Vary", "Origin");
  c.res.headers.set("Access-Control-Allow-Methods", ALLOW_METHODS);
  c.res.headers.set("Access-Control-Allow-Headers", ALLOW_HEADERS);
  c.res.headers.set("Access-Control-Allow-Private-Network", ALLOW_PRIVATE_NETWORK);
}
