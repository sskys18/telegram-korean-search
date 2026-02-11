import { useState, useEffect, useCallback } from "react";
import {
  getApiCredentials,
  saveApiCredentials,
  connectTelegram,
  requestLoginCode,
  submitLoginCode,
  submitPassword,
  startCollection,
  onCollectionProgress,
  onCollectionComplete,
  onCollectionError,
} from "../api/tauri";
import type { CollectionProgress } from "../types";

export type AuthStep =
  | "loading"
  | "setup"
  | "connecting"
  | "phone"
  | "code"
  | "2fa"
  | "collecting"
  | "ready"
  | "error";

interface AuthState {
  step: AuthStep;
  error: string | null;
  hint2fa: string | null;
  progress: CollectionProgress | null;
}

export function useAuth() {
  const [state, setState] = useState<AuthState>({
    step: "loading",
    error: null,
    hint2fa: null,
    progress: null,
  });

  // On mount: check if API credentials exist
  useEffect(() => {
    (async () => {
      try {
        const creds = await getApiCredentials();
        if (!creds) {
          setState((s) => ({ ...s, step: "setup" }));
          return;
        }
        // Credentials exist, try to connect
        setState((s) => ({ ...s, step: "connecting" }));
        const result = await connectTelegram();
        if (result.authorized) {
          setState((s) => ({ ...s, step: "ready" }));
        } else {
          setState((s) => ({ ...s, step: "phone" }));
        }
      } catch (err) {
        setState((s) => ({ ...s, step: "error", error: String(err) }));
      }
    })();
  }, []);

  // Listen for collection events
  useEffect(() => {
    const unsubs: Promise<() => void>[] = [];

    unsubs.push(
      onCollectionProgress((p) => {
        setState((s) => ({ ...s, progress: p }));
      }),
    );
    unsubs.push(
      onCollectionComplete(() => {
        setState((s) => ({ ...s, step: "ready", progress: null }));
      }),
    );
    unsubs.push(
      onCollectionError((err) => {
        setState((s) => ({ ...s, step: "error", error: err, progress: null }));
      }),
    );

    return () => {
      unsubs.forEach((p) => p.then((fn) => fn()));
    };
  }, []);

  const sendCredentials = useCallback(
    async (apiId: number, apiHash: string) => {
      try {
        setState((s) => ({ ...s, error: null, step: "connecting" }));
        await saveApiCredentials(apiId, apiHash);
        const result = await connectTelegram();
        if (result.authorized) {
          setState((s) => ({ ...s, step: "ready" }));
        } else {
          setState((s) => ({ ...s, step: "phone" }));
        }
      } catch (err) {
        setState((s) => ({ ...s, step: "setup", error: String(err) }));
      }
    },
    [],
  );

  const sendPhone = useCallback(async (phone: string) => {
    try {
      setState((s) => ({ ...s, error: null }));
      await requestLoginCode(phone);
      setState((s) => ({ ...s, step: "code" }));
    } catch (err) {
      setState((s) => ({ ...s, error: String(err) }));
    }
  }, []);

  const sendCode = useCallback(async (code: string) => {
    try {
      setState((s) => ({ ...s, error: null }));
      const result = await submitLoginCode(code);
      if (result.success) {
        setState((s) => ({ ...s, step: "collecting" }));
        await startCollection();
      } else if (result.requires_2fa) {
        setState((s) => ({ ...s, step: "2fa", hint2fa: result.hint }));
      }
    } catch (err) {
      setState((s) => ({ ...s, error: String(err) }));
    }
  }, []);

  const sendPassword = useCallback(async (password: string) => {
    try {
      setState((s) => ({ ...s, error: null }));
      await submitPassword(password);
      setState((s) => ({ ...s, step: "collecting" }));
      await startCollection();
    } catch (err) {
      setState((s) => ({ ...s, error: String(err) }));
    }
  }, []);

  return {
    ...state,
    sendCredentials,
    sendPhone,
    sendCode,
    sendPassword,
  };
}
