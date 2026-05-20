import { Context, Next } from "hono";

const ALLOW_METHODS = "POST, OPTIONS";
const ALLOW_HEADERS = "authorization, content-type";

export function openAICors(allowedOrigins: string[]) {
  const allowed = new Set(allowedOrigins);

  return async (c: Context, next: Next) => {
    const origin = c.req.header("origin");
    if (origin && allowed.has(origin)) {
      c.header("Access-Control-Allow-Origin", origin);
      c.header("Vary", "Origin");
      c.header("Access-Control-Allow-Methods", ALLOW_METHODS);
      c.header("Access-Control-Allow-Headers", ALLOW_HEADERS);
    }

    if (c.req.method === "OPTIONS") {
      return c.body(null, 204);
    }

    await next();
  };
}
