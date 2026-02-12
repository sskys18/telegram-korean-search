import { useState } from "react";
import type { AuthStep } from "../hooks/useAuth";

interface LoginPageProps {
  step: AuthStep;
  error: string | null;
  hint2fa: string | null;
  savedApiId: string;
  savedApiHash: string;
  onLogin: (apiId: number, apiHash: string, phone: string) => void;
  onCode: (code: string) => void;
  onPassword: (password: string) => void;
  onBack: () => void;
}

export function LoginPage({
  step,
  error,
  hint2fa,
  savedApiId,
  savedApiHash,
  onLogin,
  onCode,
  onPassword,
  onBack,
}: LoginPageProps) {
  const [input, setInput] = useState("");
  const [apiId, setApiId] = useState(savedApiId);
  const [apiHash, setApiHash] = useState(savedApiHash);
  const [phone, setPhone] = useState("");

  if (step === "loading" || step === "connecting" || step === "checked") {
    return (
      <div className="login-page">
        <div className="splash">
          <h1 className="splash-title">Telegram Search</h1>
          <p className="splash-subtitle">Korean message search</p>
          <div className="splash-status">
            {step === "checked" ? (
              <>
                <div className="login-checkmark">&#10003;</div>
                <p className="login-status">Authenticated</p>
              </>
            ) : (
              <>
                <div className="login-spinner" />
                <p className="login-status">
                  {step === "loading" ? "Loading..." : "Connecting..."}
                </p>
              </>
            )}
          </div>
        </div>
      </div>
    );
  }

  if (step === "error") {
    return (
      <div className="login-page">
        <p className="login-error">{error}</p>
        <button className="login-back" onClick={onBack}>
          Back
        </button>
      </div>
    );
  }

  if (step === "login") {
    const handleLogin = (e: React.FormEvent) => {
      e.preventDefault();
      const id = parseInt(apiId, 10);
      if (isNaN(id) || !apiHash.trim() || !phone.trim()) return;
      onLogin(id, apiHash.trim(), phone.trim());
    };

    return (
      <div className="login-page">
        <h2>Telegram Login</h2>
        <p className="login-detail">
          Get API credentials from{" "}
          <a
            href="https://my.telegram.org"
            target="_blank"
            rel="noopener noreferrer"
          >
            my.telegram.org
          </a>
        </p>
        {error && <p className="login-error">{error}</p>}
        <form onSubmit={handleLogin} className="login-form">
          <label className="login-label">API ID</label>
          <input
            type="text"
            value={apiId}
            onChange={(e) => setApiId(e.target.value)}
            placeholder="e.g. 12345678"
            autoFocus
          />
          <label className="login-label">API Hash</label>
          <input
            type="text"
            value={apiHash}
            onChange={(e) => setApiHash(e.target.value)}
            placeholder="e.g. a1b2c3d4e5f6..."
          />
          <label className="login-label">Phone Number</label>
          <input
            type="text"
            value={phone}
            onChange={(e) => setPhone(e.target.value)}
            placeholder="+82 10 1234 5678"
          />
          <button type="submit">Send Code</button>
        </form>
      </div>
    );
  }

  // code, 2fa steps
  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const val = input.trim();
    if (!val) return;
    if (step === "code") onCode(val);
    else if (step === "2fa") onPassword(val);
    setInput("");
  };

  const labels: Record<string, { title: string; placeholder: string }> = {
    code: { title: "Enter Verification Code", placeholder: "12345" },
    "2fa": {
      title: "Enter 2FA Password",
      placeholder: hint2fa ? `Hint: ${hint2fa}` : "Password",
    },
  };

  const { title, placeholder } = labels[step] ?? labels.code;

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
      <button className="login-back" onClick={onBack}>
        Back
      </button>
    </div>
  );
}
