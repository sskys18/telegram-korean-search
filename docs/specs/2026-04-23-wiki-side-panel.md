# Wiki Side Panel

> 2026-04-23 — supersedes the bottom-tab placement from `2026-04-23-wiki-panel-design.md`.

## Decision

Wiki UI moves out of the bottom tab strip into a right-side overlay panel.

- **Trigger**: button in the main window titlebar (top-right). Always visible, not chat-dependent.
- **Geometry**: 380px wide, full window height, overlays the right edge of `MainViewController`. Covers chat content area; chat list stays visible.
- **Dismiss**: same toggle button, or `Esc`.
- **Persistence**: open/closed state saved in `FastSettings` (key `seoyu.wiki.panel.shown`).
- **Content**: the existing `WikiTabController` (digest + list + article + toolbar) — no change to internal structure.

## Why not the bottom tab strip

Tab strip is for primary navigation peers (chats / contacts / calls / settings). Wiki is an auxiliary panel — different mental model. The placeholder icon also collided visually with `tab_contacts`, leaving users unable to find the tab.

## Implementation outline

1. Drop `wikiController: NavigationViewController?` from `MainViewController` tab list.
2. Add `wikiPanel: NSView` (containing `WikiTabController.view`) anchored to the trailing edge of `view`. Initial frame is offscreen-right; toggle animates X position.
3. Add toggle button to the main window's titlebar accessory view via `NSTitlebarAccessoryViewController`.
4. Wire toggle: read/write `FastSettings.wikiPanelShown`, animate panel slide.
5. Capture `Esc` via `NSEvent.localMonitor` while panel is open.
6. Keep `SeoyuBridge.shared.seoyu` plumbing identical — `WikiTabController` is constructed once on first show and reused.

## Out of scope

- Resizable width (defer; ship 380px fixed).
- Animated slide (acceptable to use a simple `NSAnimationContext` group; no custom transition).
- Sticky-on-resize behavior (panel snaps to current right edge on window resize via Auto Layout).
