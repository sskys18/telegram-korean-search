import { useCallback, useEffect, useState } from "react";
import {
  checkCodexAuth,
  validateCodexAuth,
  clearWikiData,
  getWikiStatus,
  onWikiWorkerError,
  onWikiWorkerProgress,
  onWikiWorkerStopped,
  reprocessWiki,
  startWikiWorker,
  stopWikiWorker,
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
  const [codexAvailable, setCodexAvailable] = useState<boolean | null>(null); // null = loading
  const [codexValid, setCodexValid] = useState<boolean | null>(null);
  const [status, setStatus] = useState<WikiStatus>(DEFAULT_STATUS);
  const [progress, setProgress] = useState<WikiProgress | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [validating, setValidating] = useState(false);

  const refreshStatus = useCallback(async () => {
    const nextStatus = await getWikiStatus();
    setStatus(nextStatus);
    if (!nextStatus.is_running) {
      setProgress(null);
    }
  }, []);

  // On mount: check Codex auth + wiki status
  useEffect(() => {
    (async () => {
      try {
        const [available, nextStatus] = await Promise.all([
          checkCodexAuth(),
          getWikiStatus(),
        ]);
        setCodexAvailable(available);
        setStatus(nextStatus);
      } catch (err) {
        setError(String(err).replace(/^Error:\s*/i, ""));
      } finally {
        setLoading(false);
      }
    })();
  }, []);

  // Event listeners
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
        refreshStatus().catch(() => {});
      }),
    );
    unsubs.push(
      onWikiWorkerStopped(() => {
        setProgress(null);
        refreshStatus().catch(() => {});
      }),
    );

    return () => {
      unsubs.forEach((promise) => {
        promise.then((fn) => fn()).catch(() => undefined);
      });
    };
  }, [refreshStatus]);

  const handleValidate = useCallback(async () => {
    setValidating(true);
    setError(null);
    try {
      const valid = await validateCodexAuth();
      setCodexValid(valid);
      if (!valid) {
        setError("Codex OAuth token is invalid or expired. Run 'codex login' in terminal.");
      }
    } catch (err) {
      setCodexValid(false);
      setError(String(err).replace(/^Error:\s*/i, ""));
    } finally {
      setValidating(false);
    }
  }, []);

  const handleStart = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      await startWikiWorker();
      await refreshStatus();
    } catch (err) {
      setError(String(err).replace(/^Error:\s*/i, ""));
    } finally {
      setBusy(false);
    }
  }, [refreshStatus]);

  const handleStop = useCallback(async () => {
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

  const handleReprocess = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      await reprocessWiki();
      await refreshStatus();
    } catch (err) {
      setError(String(err).replace(/^Error:\s*/i, ""));
    } finally {
      setBusy(false);
    }
  }, [refreshStatus]);

  const handleClear = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      await clearWikiData();
      await refreshStatus();
    } catch (err) {
      setError(String(err).replace(/^Error:\s*/i, ""));
    } finally {
      setBusy(false);
    }
  }, [refreshStatus]);

  return {
    codexAvailable,
    codexValid,
    status,
    progress,
    loading,
    busy,
    error,
    validating,
    validateCodex: handleValidate,
    startWorker: handleStart,
    stopWorker: handleStop,
    reprocessWiki: handleReprocess,
    clearWikiData: handleClear,
    refreshStatus,
  };
}
