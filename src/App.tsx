import { useState } from "react";
import { SearchPage } from "./pages/SearchPage";
import { LoginPage } from "./pages/LoginPage";
import { WikiPage } from "./pages/WikiPage";
import { TabBar } from "./components/TabBar";
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

  const [activeTab, setActiveTab] = useState<"search" | "wiki">("search");

  if (step === "ready") {
    return (
      <div className="app-shell">
        <TabBar activeTab={activeTab} onTabChange={setActiveTab} />
        {activeTab === "search" ? (
          <SearchPage syncing={syncing} progress={progress} />
        ) : (
          <WikiPage />
        )}
      </div>
    );
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
