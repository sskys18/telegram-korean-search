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
    sendCredentials,
    sendPhone,
    sendCode,
    sendPassword,
  } = useAuth();

  if (step === "ready") {
    return <SearchPage />;
  }

  return (
    <LoginPage
      step={step}
      error={error}
      hint2fa={hint2fa}
      progress={progress}
      onCredentials={sendCredentials}
      onPhone={sendPhone}
      onCode={sendCode}
      onPassword={sendPassword}
    />
  );
}

export default App;
