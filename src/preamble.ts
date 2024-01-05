/* eslint-disable @typescript-eslint/no-unused-vars */

type Method = "DELETE" | "GET" | "PUT" | "POST" | "HEAD" | "TRACE" | "PATCH";

let GLOBAL_API_BASE = "";
export const getApiBase = (options?: ApiOptions) =>
  options?.apiBase ?? GLOBAL_API_BASE;
export const setGlobalApiBase = (apiBase: string) =>
  (GLOBAL_API_BASE = apiBase);

export type ApiOptions = {
  fetch?: typeof fetch;
  apiBase?: string;
  headers?: Record<string, string>;
};

export const requestPlain = (
  method: Method,
  url: string,
  body?: unknown,
  options?: ApiOptions
): {
  data: Promise<string>;
  cancel: (reason?: string) => void;
} => {
  let inFlight = true;
  const controller = new AbortController();
  const data = (options?.fetch ?? fetch)(`${getApiBase(options)}${url}`, {
    method: method.toUpperCase(),
    body: typeof body != "undefined" ? JSON.stringify(body) : void 0,
    signal: controller.signal,
    headers: {
      ...options?.headers,
      ...(typeof body != "undefined"
        ? { "Content-Type": "application/json" }
        : {}),
    },
  }).then(async (res) => {
    inFlight = false;
    if (res.ok) {
      const text = await res.text();
      try {
        return text;
      } catch (_) {
        throw text;
      }
    } else {
      throw res.text();
    }
  });

  return {
    data,
    cancel: (reason) => {
      if (inFlight) controller.abort(reason);
    },
  };
};

export const requestJson = <T>(
  method: Method,
  url: string,
  body?: unknown,
  options: ApiOptions = {}
): {
  data: Promise<T>;
  cancel: (reason?: string) => void;
} => {
  const { data, cancel } = requestPlain(method, url, body, options);
  return { data: data.then((text) => JSON.parse(text) as T), cancel };
};

export type SSEStream<T> = (
  event:
    | { type: "message"; data: T }
    | {
        type: "error";
        event: Event;
      }
) => void;

const sse = <T>(
  _method: Method,
  url: string,
  options?: ApiOptions
): {
  cancel: () => void;
  listen: (stream: SSEStream<T>) => void;
} => {
  const source = new EventSource(`${getApiBase(options)}${url}`);

  let stream: SSEStream<T> | null = null;

  source.onmessage = (event) => {
    const data = event.data;
    stream?.({ type: "message", data });
  };
  source.onerror = (event) => {
    stream?.({ type: "error", event });
  };
  return {
    cancel: () => source.close(),
    listen: (newStream) => (stream = newStream),
  };
};
