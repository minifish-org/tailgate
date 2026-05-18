import { Context } from "hono";

export class GatewayError extends Error {
  constructor(
    message: string,
    public readonly status: number,
    public readonly code: string,
  ) {
    super(message);
  }
}

export function errorResponse(c: Context, status: number, code: string, message: string) {
  return c.json(
    {
      error: {
        message,
        type: "gateway_error",
        code,
      },
    },
    status as never,
  );
}

export function messageFromUnknown(error: unknown): string {
  if (error instanceof Error) return error.message;
  return String(error);
}
