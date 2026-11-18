// The Operator API over grpc-web. The Server serves this page, so calls are same-origin and the
// session cookie (HttpOnly, set by Login) rides along automatically.
import { Code, ConnectError, createClient } from "@connectrpc/connect";
import { createGrpcWebTransport } from "@connectrpc/connect-web";
import { timestampDate, type Timestamp } from "@bufbuild/protobuf/wkt";
import { OperatorService } from "./gen/rustyrmm/operator/v1/operator_pb";

export const api = createClient(
  OperatorService,
  createGrpcWebTransport({ baseUrl: window.location.origin }),
);

/** True when the call failed because there is no valid session. */
export function needsLogin(err: unknown): boolean {
  return ConnectError.from(err).code === Code.Unauthenticated;
}

export function message(err: unknown): string {
  return ConnectError.from(err).rawMessage;
}

export function when(ts: Timestamp | undefined): string {
  if (!ts) return "never";
  return timestampDate(ts).toLocaleString();
}

/** Every page of a paged list call. */
export async function all<T>(
  page: (token: string) => Promise<{ items: T[]; next: string }>,
): Promise<T[]> {
  const out: T[] = [];
  let token = "";
  do {
    const r = await page(token);
    out.push(...r.items);
    token = r.next;
  } while (token);
  return out;
}
