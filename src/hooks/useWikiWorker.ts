import { useCallback, useEffect, useState } from "react";
import {
  clearWikiData,
  getOpenaiApiKey,
  getWikiStatus,
  onWikiWorkerError,
  onWikiWorkerProgress,
  onWikiWorkerStopped,
  reprocessWiki,
  saveOpenaiApiKey,
  startWikiWorker,
  stopWikiWorker,
  validateOpenaiApiKey,
} from "../api/tauri";
import type { WikiProgress, WikiStatus } from "../types";

const DEFAULT_STATUS: WikiStatus = {
  pending: 0,
  processing: 0,
  done: 0,
  failed: 0,
  skipped: 0,
  topics_count: 0,
  is_running: false,
};

export function useWikiWorker() {
  const [apiKey, setApiKey] = useState("");
  const [savedKeyMask, setSavedKeyMask] = useState<string | null>(null);
  const [status, setStatus] = useState<WikiStatus>(DEFAULT_STATUS);
  const [progress, setProgress] = useState<WikiProgress | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [validationMessage, setValidationMessage] = useState<string | null>(null);
  const [validating, setValidating] = useState(false);

  const refreshStatus = useCallback(async () => {
    const nextStatus = await getWikiStatus();
    setStatus(nextStatus);
    if (!nextStatus.is_running) {
      setProgress(null);
    }
  }, []);

  useEffect(() => {
    (async () => {
      try {
        const [maskedKey, nextStatus] = await Promise.all([
          getOpenaiApiKey(),
          getWikiStatus(),
        ]);
        setSavedKeyMask(maskedKey);
        setStatus(nextStatus);
      } catch (err) {
        setError(String(err).replace(/^Error:\s*/i, ""));
      } finally {
        setLoading(false);
      }
    })();
  }, []);

  useEffect(() => {
    const unsubs: Promise<() => void>[] = [];

    unsubs.push(
      onWikiWorkerProgress((nextProgress) => {
        setProgress(nextProgress);
        setStatus((prev) => ({
          ...prev,
          pending: nextProgress.queue_remaining,
          done: nextProgress.processed,
          is_running: true,
        }));
      }),
    );
    unsubs.push(
      onWikiWorkerError((event) => {
        setError(event.message);
        refreshStatus().catch((err) =>
          console.error("Failed to refresh wiki status:", err),
        );
      }),
    );
    unsubs.push(
      onWikiWorkerStopped(() => {
        setProgress(null);
        refreshStatus().catch((err) =>
          console.error("Failed to refresh wiki status:", err),
        );
      }),
    );

    return () => {
      unsubs.forEach((promise) => {
        promise.then((fn) => fn()).catch(() => undefined);
      });
    };
  }, [refreshStatus]);

  const saveApiKey = useCallback(async () => {
    const trimmed = apiKey.trim();
    if (!trimmed) {
      setError("Enter an OpenAI API key.");
      return false;
    }

    setBusy(true);
    setError(null);
    try {
      await saveOpenaiApiKey(trimmed);
      const maskedKey = await getOpenaiApiKey();
      setSavedKeyMask(maskedKey);
      setValidationMessage("API key saved.");
      return true;
    } catch (err) {
      setError(String(err).replace(/^Error:\s*/i, ""));
      return false;
    } finally {
      setBusy(false);
    }
  }, [apiKey]);

  const validateApiKey = useCallback(async () => {
    const trimmed = apiKey.trim();
    if (!trimmed) {
      setValidationMessage("Enter an API key to validate.");
      return false;
    }

    setValidating(true);
    setValidationMessage(null);
    setError(null);
    try {
      const valid = await validateOpenaiApiKey(trimmed);
      setValidationMessage(valid ? "API key is valid." : "API key validation failed.");
      return valid;
    } catch (err) {
      setValidationMessage(String(err).replace(/^Error:\s*/i, ""));
      return false;
    } finally {
      setValidating(false);
    }
  }, [apiKey]);

  const startWorker = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      if (apiKey.trim()) {
        await saveOpenaiApiKey(apiKey.trim());
        const maskedKey = await getOpenaiApiKey();
        setSavedKeyMask(maskedKey);
      }
      await startWikiWorker();
      await refreshStatus();
    } catch (err) {
      setError(String(err).replace(/^Error:\s*/i, ""));
    } finally {
      setBusy(false);
    }
  }, [apiKey, refreshStatus]);

  const stopWorker = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      await stopWikiWorker();
      await refreshStatus();
    } catch (err) {
      setError(String(err).replace(/^Error:\s*/i, ""));
    } finally {
      setBusy(false);
    }
  }, [refreshStatus]);

  const reprocessAll = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      await reprocessWiki();
      await refreshStatus();
      setValidationMessage("Wiki queue reset and re-enqueued.");
    } catch (err) {
      setError(String(err).replace(/^Error:\s*/i, ""));
    } finally {
      setBusy(false);
    }
  }, [refreshStatus]);

  const clearAll = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      await clearWikiData();
      await refreshStatus();
      setValidationMessage("Wiki data cleared.");
    } catch (err) {
      setError(String(err).replace(/^Error:\s*/i, ""));
    } finally {
      setBusy(false);
    }
  }, [refreshStatus]);

  return {
    apiKey,
    setApiKey,
    savedKeyMask,
    status,
    progress,
    loading,
    busy,
    error,
    validationMessage,
    validating,
    saveApiKey,
    validateApiKey,
    startWorker,
    stopWorker,
    reprocessWiki: reprocessAll,
    clearWikiData: clearAll,
    refreshStatus,
  };
}
