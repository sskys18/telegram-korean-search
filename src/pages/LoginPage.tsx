import { useState } from "react";
import type { AuthStep } from "../hooks/useAuth";
import type { CollectionProgress } from "../types";

interface LoginPageProps {
  step: AuthStep;
  error: string | null;
  hint2fa: string | null;
  progress: CollectionProgress | null;
  onCredentials: (apiId: number, apiHash: string) => void;
  onPhone: (phone: string) => void;
  onCode: (code: string) => void;
  onPassword: (password: string) => void;
}

export function LoginPage({
  step,
  error,
  hint2fa,
  progress,
  onCredentials,
  onPhone,
  onCode,
  onPassword,
}: LoginPageProps) {
  const [input, setInput] = useState("");
  const [apiId, setApiId] = useState("");
  const [apiHash, setApiHash] = useState("");

  if (step === "loading" || step === "connecting") {
    return (
      <div className="login-page">
        <div className="login-spinner" />
        <p className="login-status">
          {step === "loading" ? "Loading..." : "Connecting to Telegram..."}
        </p>
      </div>
    );
  }

  if (step === "collecting") {
    return (
      <div className="login-page">
        <div className="login-spinner" />
        <p className="login-status">Syncing messages...</p>
        {progress && (
          <p className="login-detail">
            {progress.phase === "chats"
              ? progress.detail
              : `${progress.chat_title} (${(progress.chats_done ?? 0) + 1}/${progress.chats_total})`}
          </p>
        )}
      </div>
    );
  }

  if (step === "error") {
    return (
      <div className="login-page">
        <p className="login-error">{error}</p>
      </div>
    );
  }

  if (step === "setup") {
    const handleSetup = (e: React.FormEvent) => {
      e.preventDefault();
      const id = parseInt(apiId, 10);
      if (isNaN(id) || !apiHash.trim()) return;
      onCredentials(id, apiHash.trim());
    };

    return (
      <div className="login-page">
        <h2>Telegram API Setup</h2>
        <p className="login-detail">
          Get your API credentials from{" "}
          <a
            href="https://my.telegram.org"
            target="_blank"
            rel="noopener noreferrer"
          >
            my.telegram.org
          </a>
        </p>
        {error && <p className="login-error">{error}</p>}
        <form onSubmit={handleSetup} className="login-form">
          <input
            type="text"
            value={apiId}
            onChange={(e) => setApiId(e.target.value)}
            placeholder="API ID"
            autoFocus
          />
          <input
            type="text"
            value={apiHash}
            onChange={(e) => setApiHash(e.target.value)}
            placeholder="API Hash"
          />
          <button type="submit">Continue</button>
        </form>
      </div>
    );
  }

  // phone, code, 2fa steps
  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const val = input.trim();
    if (!val) return;
    if (step === "phone") onPhone(val);
    else if (step === "code") onCode(val);
    else if (step === "2fa") onPassword(val);
    setInput("");
  };

  const labels: Record<string, { title: string; placeholder: string }> = {
    phone: { title: "Enter Phone Number", placeholder: "+82 10 1234 5678" },
    code: { title: "Enter Verification Code", placeholder: "12345" },
    "2fa": {
      title: "Enter 2FA Password",
      placeholder: hint2fa ? `Hint: ${hint2fa}` : "Password",
    },
  };

  const { title, placeholder } = labels[step] ?? labels.phone;

  return (
    <div className="login-page">
      <h2>{title}</h2>
      {error && <p className="login-error">{error}</p>}
      <form onSubmit={handleSubmit} className="login-form">
        <input
          type={step === "2fa" ? "password" : "text"}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          placeholder={placeholder}
          autoFocus
        />
        <button type="submit">Continue</button>
      </form>
    </div>
  );
}
