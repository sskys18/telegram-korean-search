import { SearchPage } from "./pages/SearchPage";
import { LoginPage } from "./pages/LoginPage";
import { useAuth } from "./hooks/useAuth";
import "./App.css";

function App() {
  const {
    step,
    error,
    hint2fa,
    progress,
    syncing,
    savedApiId,
    savedApiHash,
    sendLogin,
    sendCode,
    sendPassword,
    goBack,
  } = useAuth();

  if (step === "ready") {
    return <SearchPage syncing={syncing} progress={progress} />;
  }

  return (
    <LoginPage
      step={step}
      error={error}
      hint2fa={hint2fa}
      savedApiId={savedApiId}
      savedApiHash={savedApiHash}
      onLogin={sendLogin}
      onCode={sendCode}
      onPassword={sendPassword}
      onBack={goBack}
    />
  );
}

export default App;
