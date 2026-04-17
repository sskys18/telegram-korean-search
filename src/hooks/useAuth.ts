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
  readEnvCredentials,
} from "../api/tauri";
import type { CollectionProgress } from "../types";

export type AuthStep =
  | "loading"
  | "login"
  | "connecting"
  | "code"
  | "2fa"
  | "ready"
  | "error";

function friendlyError(err: unknown): string {
  const msg = String(err);
  if (msg.includes("API credentials not configured"))
    return "Please enter your API credentials first.";
  if (msg.includes("PHONE_NUMBER_INVALID"))
    return "Invalid phone number. Please include the country code (e.g. +82).";
  if (msg.includes("PHONE_CODE_INVALID"))
    return "Incorrect verification code. Please try again.";
  if (msg.includes("PHONE_CODE_EXPIRED"))
    return "Verification code expired. Please request a new one.";
  if (msg.includes("PASSWORD_HASH_INVALID"))
    return "Incorrect password. Please try again.";
  if (msg.includes("FLOOD_WAIT") || msg.includes("FLOOD"))
    return "Too many attempts. Please wait a few minutes and try again.";
  if (msg.includes("AUTH_KEY_UNREGISTERED"))
    return "Session expired. Please log in again.";
  if (msg.includes("network") || msg.includes("connection"))
    return "Network error. Please check your internet connection.";
  // Strip "Error: " prefix from Tauri invoke errors
  return msg.replace(/^Error:\s*/i, "");
}

interface AuthState {
  step: AuthStep;
  error: string | null;
  hint2fa: string | null;
  progress: CollectionProgress | null;
  savedApiId: string;
  savedApiHash: string;
  savedPhone: string;
}

function transitionToReady(
  setState: React.Dispatch<React.SetStateAction<AuthState>>,
) {
  setState((s) => ({ ...s, step: "ready" }));
  startCollection().catch((err) =>
    console.error("Collection failed:", err),
  );
}

export function useAuth() {
  const [state, setState] = useState<AuthState>({
    step: "loading",
    error: null,
    hint2fa: null,
    progress: null,
    savedApiId: "",
    savedApiHash: "",
    savedPhone: "",
  });

  // On mount: check if API credentials exist.
  // StrictMode guard: cancelled flag prevents the stale mount from
  // calling requestLoginCode a second time (which would FLOOD_WAIT).
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const creds = await getApiCredentials();
        const env = await readEnvCredentials().catch(() => null);
        const envApiId = env?.api_id ?? "";
        const envApiHash = env?.api_hash ?? "";
        const envPhone = env?.phone ?? "";

        if (!creds) {
          // No saved credentials — auto-login if all 3 .env values present
          if (envApiId && envApiHash && envPhone) {
            const id = parseInt(envApiId, 10);
            if (!isNaN(id)) {
              setState((s) => ({
                ...s,
                step: "connecting",
                savedApiId: envApiId,
                savedApiHash: envApiHash,
                savedPhone: envPhone,
              }));
              try {
                await saveApiCredentials(id, envApiHash);
                await connectTelegram();
                if (cancelled) return;
                const normalized = envPhone
                  .replace(/[^\d+]/g, "")
                  .replace(/(?!^)\+/g, "");
                await requestLoginCode(normalized);
                if (cancelled) return;
                setState((s) => ({
                  ...s,
                  step: "code",
                  savedApiId: envApiId,
                  savedApiHash: envApiHash,
                  savedPhone: envPhone,
                }));
                return;
              } catch (err) {
                if (cancelled) return;
                setState((s) => ({
                  ...s,
                  step: "login",
                  error: friendlyError(err),
                  savedApiId: envApiId,
                  savedApiHash: envApiHash,
                  savedPhone: envPhone,
                }));
                return;
              }
            }
          }

          setState((s) => ({
            ...s,
            step: "login",
            savedApiId: envApiId,
            savedApiHash: envApiHash,
            savedPhone: envPhone,
          }));
          return;
        }

        // Credentials exist in DB — try to connect
        setState((s) => ({
          ...s,
          step: "connecting",
          savedApiId: String(creds.api_id),
          savedApiHash: creds.api_hash,
          savedPhone: envPhone,
        }));
        const result = await connectTelegram();
        if (cancelled) return;
        if (result.authorized) {
          transitionToReady(setState);
        } else if (envPhone) {
          // Not authenticated — auto-request code using .env phone
          try {
            const normalized = envPhone
              .replace(/[^\d+]/g, "")
              .replace(/(?!^)\+/g, "");
            await requestLoginCode(normalized);
            if (cancelled) return;
            setState((s) => ({
              ...s,
              step: "code",
              savedPhone: envPhone,
            }));
          } catch (err) {
            if (cancelled) return;
            setState((s) => ({
              ...s,
              step: "login",
              error: friendlyError(err),
              savedPhone: envPhone,
            }));
          }
        } else {
          setState((s) => ({ ...s, step: "login" }));
        }
      } catch (err) {
        if (cancelled) return;
        setState((s) => ({ ...s, step: "error", error: friendlyError(err) }));
      }
    })();
    return () => { cancelled = true; };
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
        setState((s) => ({ ...s, progress: null }));
      }),
    );
    unsubs.push(
      onCollectionError((err) => {
        console.error("Collection error:", err);
        setState((s) => ({ ...s, progress: null }));
      }),
    );

    return () => {
      unsubs.forEach((p) => p.then((fn) => fn()));
    };
  }, []);

  const sendLogin = useCallback(
    async (apiId: number, apiHash: string, phone: string) => {
      try {
        setState((s) => ({ ...s, error: null, step: "connecting" }));
        await saveApiCredentials(apiId, apiHash);
        await connectTelegram();
        const normalized = phone
          .replace(/[^\d+]/g, "")
          .replace(/(?!^)\+/g, "");
        await requestLoginCode(normalized);
        setState((s) => ({
          ...s,
          step: "code",
          savedApiId: String(apiId),
          savedApiHash: apiHash,
        }));
      } catch (err) {
        setState((s) => ({ ...s, step: "login", error: friendlyError(err) }));
      }
    },
    [],
  );

  const sendCode = useCallback(async (code: string) => {
    try {
      setState((s) => ({ ...s, error: null, step: "connecting" }));
      const result = await submitLoginCode(code);
      if (result.success) {
        transitionToReady(setState);
      } else if (result.requires_2fa) {
        setState((s) => ({ ...s, step: "2fa", hint2fa: result.hint }));
      }
    } catch (err) {
      setState((s) => ({ ...s, step: "code", error: friendlyError(err) }));
    }
  }, []);

  const sendPassword = useCallback(async (password: string) => {
    try {
      setState((s) => ({ ...s, error: null, step: "connecting" }));
      await submitPassword(password);
      transitionToReady(setState);
    } catch (err) {
      // grammers consumes PasswordToken by value — after a failed attempt
      // (wrong password or missing token), we must restart the sign_in flow.
      // Re-request login code so the user can re-enter code → 2FA → password.
      try {
        const env = await readEnvCredentials().catch(() => null);
        const phone = env?.phone;
        if (phone) {
          const normalized = phone
            .replace(/[^\d+]/g, "")
            .replace(/(?!^)\+/g, "");
          await requestLoginCode(normalized);
          setState((s) => ({
            ...s,
            step: "code",
            error: "비밀번호 오류. 인증 코드를 다시 입력해주세요.",
          }));
          return;
        }
      } catch {
        // requestLoginCode failed — fall through to login
      }
      setState((s) => ({ ...s, step: "login", error: friendlyError(err) }));
    }
  }, []);

  const goBack = useCallback(() => {
    setState((s) => {
      switch (s.step) {
        case "code":
        case "2fa":
          return { ...s, step: "login" as AuthStep, error: null, hint2fa: null };
        case "error":
          return { ...s, step: "login" as AuthStep, error: null };
        default:
          return s;
      }
    });
  }, []);

  return {
    ...state,
    syncing: state.progress !== null,
    sendLogin,
    sendCode,
    sendPassword,
    goBack,
  };
}
