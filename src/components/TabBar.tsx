interface TabBarProps {
  activeTab: "search" | "wiki";
  onTabChange: (tab: "search" | "wiki") => void;
}

export function TabBar({ activeTab, onTabChange }: TabBarProps) {
  return (
    <div className="tab-bar" role="tablist" aria-label="Main navigation">
      <button
        type="button"
        role="tab"
        aria-selected={activeTab === "search"}
        className={activeTab === "search" ? "tab-button tab-active" : "tab-button"}
        onClick={() => onTabChange("search")}
      >
        Search
      </button>
      <button
        type="button"
        role="tab"
        aria-selected={activeTab === "wiki"}
        className={activeTab === "wiki" ? "tab-button tab-active" : "tab-button"}
        onClick={() => onTabChange("wiki")}
      >
        Wiki
      </button>
    </div>
  );
}
