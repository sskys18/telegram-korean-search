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

  if (step === "loading" || step === "connecting") {
    return (
      <div className="login-page">
        <div className="splash">
          <h1 className="splash-title">텔레그램 한국어 검색</h1>
          <p className="splash-subtitle">텔레그램 메시지를 한국어로 검색하세요</p>
          <div className="splash-status">
            <div className="login-spinner" />
            <p className="login-status">
              {step === "loading" ? "로딩 중..." : "연결 중..."}
            </p>
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
          뒤로
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
        <h2>텔레그램 로그인</h2>
        <p className="login-detail">
          API 인증 정보를 여기서 발급받으세요:{" "}
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
          <label className="login-label">전화번호</label>
          <input
            type="text"
            value={phone}
            onChange={(e) => setPhone(e.target.value)}
            placeholder="+82 10 1234 5678"
          />
          <button type="submit">인증 코드 발송</button>
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
    code: { title: "인증 코드 입력", placeholder: "12345" },
    "2fa": {
      title: "2FA 비밀번호 입력",
      placeholder: hint2fa ? `힌트: ${hint2fa}` : "비밀번호",
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
        <button type="submit">계속</button>
      </form>
      <button className="login-back" onClick={onBack}>
        뒤로
      </button>
    </div>
  );
}
