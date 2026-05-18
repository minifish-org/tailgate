import { Context, Next } from "hono";
import { GatewayError } from "./errors.js";

export async function requireRouterAuth(c: Context, next: Next) {
  const expected = process.env.ROUTER_API_KEY;
  if (!expected) {
    throw new GatewayError("ROUTER_API_KEY is not configured", 500, "router_key_not_configured");
  }

  const authorization = c.req.header("authorization");
  if (!authorization?.startsWith("Bearer ")) {
    throw new GatewayError("Missing or invalid Authorization header", 401, "unauthorized");
  }

  const token = authorization.slice("Bearer ".length);
  if (token !== expected) {
    throw new GatewayError("Invalid API key", 401, "unauthorized");
  }

  await next();
}
