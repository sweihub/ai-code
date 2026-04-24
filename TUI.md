# TUI Architecture & Layout Guide

> This document captures the complete UI layout structure, component relationships, and critical implementation details for the Rust TUI (translated from `~/claudecode/openclaudecode`).

## Core Principle

**The Rust TUI must match the TypeScript React layout exactly.** The TypeScript uses Ink (React-based terminal UI), and we translate to ratatui. Component hierarchy, positioning, and scrolling behavior must be identical.

---

## 1. FullscreenLayout Structure

### TypeScript Source: `~/claudecode/openclaudecode/src/components/FullscreenLayout.tsx`

```tsx
<PromptOverlayProvider>
  {/* Main scrollable region - fills screen */}
  <Box flexGrow={1} flexDirection="column" overflow="hidden">
    {headerPrompt}  {/* StickyPromptHeader - 1 row FIXED */}
    <ScrollBox ref={scrollRef} flexGrow={1} paddingTop={padCollapsed?0:1} stickyScroll={true}>
      <ScrollChromeContext>{scrollable}</ScrollChromeContext>
      {overlay}
    </ScrollBox>
    {pill}  {/* NewMessagesPill - absolute bottom overlay */}
    {bottomFloat}  {/* Companion bubble - absolute bottom-right */}
  </Box>
  
  {/* Bottom fixed section - never scrolls */}
  <Box flexDirection="column" flexShrink={0} width="100%" maxHeight="50%">
    {SuggestionsOverlay}
    {DialogOverlay}
    <Box flexDirection="column" flexGrow={1} overflowY="hidden">
      {bottom}  {/* PromptInput + footer */}
    </Box>
  </Box>
  
  {/* Modal overlay */}
  {modal}
</PromptOverlayProvider>
```

### Rust Translation: `src/tui/widgets/fullscreen_layout.rs`

```rust
render_fullscreen_layout(
    frame, area,
    // 1. scrollable content (messages + streaming + spinner)
    |f, area| render_scrollable_content(...),
    // 2. bottom content (PromptInput - fixed at screen bottom)
    |f, area| render_bottom(...),
    // 3. overlay (PermissionRequest)
    render_overlay,
    // 4. bottom_float (companion sprite)
    render_bottom_float,
    // 5. modal (dialogs)
    render_modal,
    // State
    &scroll_handle, divider_y, &chrome_context, &config,
)
```

---

## 2. CRITICAL: Spinner vs Input Separation

### The spinner is NOT bundled with the input box!

**TypeScript Structure:**
```tsx
<FullscreenLayout
  scrollable={
    <Messages />
    <UserTextMessage placeholder />
    <Box flexGrow={1} />          {/* Spacer pushes spinner down */}
    <SpinnerWithVerb />           {/* ← In scrollable area */}
  }
  bottom={
    <PromptInput                  {/* ← Fixed at screen bottom */}
      borderStyle="round"
      borderBottom
    />
    <PromptInputFooter />
  }
/>
```

**Behavior:**
- **Spinner**: Inside `scrollable` slot → scrolls WITH content
- **Input**: Inside `bottom` slot → FIXED at screen bottom, never moves
- They do NOT move together

**Visual Layout:**
```
┌─────────────────────────────────┐
│ Messages                        │ ← Scrollable
│ User text                       │
│ Assistant response              │
│                                 │ ← <Box flexGrow={1} />
│ ⠋ Working... (5s) ↓ 150 tokens  │ ← Spinner (scrolls with content)
├─────────────────────────────────┤
│ ❯ Type a message...             │ ← PromptInput (fixed)
└─────────────────────────────────┘
```

**When user scrolls up:**
- Spinner moves UP with messages
- Input stays FIXED at bottom
- User can see previous conversation

**When at bottom (sticky scroll):**
- Spinner appears just above input
- Input remains at screen bottom

---

## 3. Scroll System

### ScrollBox (TypeScript) → ScrollBox (Rust)

**TypeScript Source:** `~/claudecode/openclaudecode/src/ink/components/ScrollBox.tsx`

**Key Properties:**
```typescript
interface ScrollBoxHandle {
  scrollTo(y: number): void;
  scrollBy(dy: number): void;
  scrollToBottom(): void;
  getScrollTop(): number;
  getPendingDelta(): number;
  getScrollHeight(): number;
  getViewportHeight(): number;
  isSticky(): boolean;
  subscribe(listener: () => void): () => void;
  setClampBounds(min: number | undefined, max: number | undefined): void;
}
```

**Rust Translation:** `src/tui/widgets/scroll_box.rs`
```rust
pub struct ScrollBoxHandle {
    pub scroll_top: u16,
    pub scroll_height: u16,
    pub viewport_height: u16,
    pub sticky_scroll: bool,
    pub pending_delta: i32,
    pub scroll_clamp_min: Option<u16>,
    pub scroll_clamp_max: Option<u16>,
    listeners: Rc<RefCell<Vec<Box<dyn Fn()>>>>,
}
```

### Virtual Scroll

**TypeScript Source:** `~/claudecode/openclaudecode/src/hooks/useVirtualScroll.ts`

**Constants:**
```typescript
const DEFAULT_ESTIMATE = 3;      // Unmeasured item height
const OVERSCAN_ROWS = 80;        // Extra rows above/below viewport
const COLD_START_COUNT = 30;     // Items before layout
const SCROLL_QUANTUM = 40;       // scrollTop quantization
const PESSIMISTIC_HEIGHT = 1;    // Worst-case unmeasured height
const MAX_MOUNTED_ITEMS = 300;   // Cap on mounted items
const SLIDE_STEP = 25;           // Max new items per commit
```

**Rust Implementation:** `src/hooks/use_virtual_scroll.rs`

**Key Logic:**
1. Mount only items in viewport + overscan
2. Spacer boxes hold scroll height constant
3. Height cache populated by Yoga layout measurements
4. Binary search for start position (O(log n), not linear)
5. Slide cap limits mount rate during fast scroll

### Sticky Scroll Behavior

**When sticky_scroll = true:**
- Automatically scrolls to bottom when content grows
- Set by: `scrollToBottom()`, initial attribute, renderer positional follow
- Cleared by: `scrollTo()`, `scrollBy()`, user manual scroll

**Rust State:** `src/tui/app.rs`
```rust
pub struct App {
    pub sticky_scroll: bool,        // Auto-scroll enabled
    pub scroll_offset: usize,       // Current scroll position
    pub prev_message_count: usize,  // For detecting content growth
}
```

---

## 4. Auto-Scroll Logic

**TypeScript:** `stickyScroll && (grew || is_loading)`

**Rust:** `src/tui/repl_main_render.rs`
```rust
let should_auto_scroll = app.sticky_scroll && is_loading;

if should_auto_scroll && target_scroll != app.scroll_offset {
    app.scroll_offset = target_scroll;
}
```

**Key Points:**
- Only update scroll when `sticky_scroll = true`
- User manual scroll sets `sticky_scroll = false`
- Content growth detected during loading
- Target scroll = `total_lines.saturating_sub(viewport_height)`

---

## 5. Component Rendering Order

### REPL Main Render: `src/tui/repl_main_render.rs`

```
1. Calculate layout (scroll_h, bottom_y, bottom_h)
2. Render scrollable area:
   - Messages (with scroll offset)
   - Placeholder text (if loading)
   - Streaming text (live preview)
   - Spinner (at bottom of content)
3. Render bottom area:
   - Buddy (if narrow terminal)
   - Task list (if expanded)
   - Prompt input (with borders)
   - Suggestions (if showing)
4. Render modal divider (if dialog focused)
5. Render dialogs
```

### Spinner Rendering

**TypeScript Source:** `REPL.tsx` line ~4641
```tsx
{showSpinner && <SpinnerWithVerb mode={streamMode} ... />}
```

**Rust:** `src/tui/repl_main_render.rs`
```rust
// Spinner renders at: area.y + y (where content ended)
if is_loading {
    let spinner_y = area.y + y;
    // Render spinner with shimmer animation
}
```

---

## 6. Message Rendering

### Message Registry System

**Location:** `src/tui/widgets/message_renderers/`

**Types:**
- User messages
- Assistant messages
- System messages
- Tool use messages
- Progress messages

**Line Count Calculation:**
```rust
let lc = registry
    .find_renderer(msg)
    .map(|r| r.line_count(msg, width))
    .unwrap_or_else(|| msg.content.lines().count().max(1));
```

### Streaming Text

**State:** `app.streaming_text`
- Accumulates text delta during streaming
- Rendered with yellow color, "❯" prefix
- Cleared on `ContentBlockStart`

### Streaming Thinking

**State:** `app.streaming_thinking`
- Shown AFTER streaming ends
- 30-second timeout after streaming ended
- Uses `AssistantThinkingMessage` widget

---

## 7. Dialog System

### 7.1 Dialog Overlay Architecture

**TypeScript Source:** `src/context/promptOverlayContext.tsx`

The dialog system uses a **two-channel portal** mechanism to escape `FullscreenLayout`'s bottom-slot `overflowY:hidden` clip:

```tsx
<PromptOverlayProvider>
  <DataContext.Provider value={data}>        {/* Suggestion data (structured) */}
    <DialogContext.Provider value={dialog}> {/* Arbitrary dialog node */}
      {children}
    </DialogContext.Provider>
  </DataContext.Provider>
</PromptOverlayProvider>
```

**Two channels:**
1. `useSetPromptOverlay()` — slash-command suggestion data (written by `PromptInputFooter`)
2. `useSetPromptOverlayDialog()` — arbitrary dialog node (written by `PromptInput`, `AutoModeOptInDialog`, etc.)

Both are rendered by `FullscreenLayout` outside the clipped slot:

```tsx
// SuggestionsOverlay — absolute bottom="100%" of the bottom slot
function SuggestionsOverlay() {
  const data = usePromptOverlay();
  return (
    <Box position="absolute" bottom="100%" left={0} right={0}
         paddingX={2} paddingTop={1} flexDirection="column" opaque={true}>
      <PromptInputFooterSuggestions ... />
    </Box>
  );
}

// DialogOverlay — same clip-escape pattern, paints over suggestions
function DialogOverlay() {
  const node = usePromptOverlayDialog();
  return (
    <Box position="absolute" bottom="100%" left={0} right={0} opaque={true}>
      {node}
    </Box>
  );
}
```

### 7.2 Modal Dialog (slash-command dialogs)

**TypeScript Source:** `src/components/FullscreenLayout.tsx` lines ~420-430

The **modal** slot renders a bottom-anchored panel with a ▔▔▔ divider:

```tsx
{modal != null &&
  <ModalContext value={{
    rows: terminalRows - MODAL_TRANSCRIPT_PEEK - 1,   // MODAL_TRANSCRIPT_PEEK = 2
    columns: columns - 4,
    scrollRef: modalScrollRef ?? null
  }}>
    <Box position="absolute" bottom={0} left={0} right={0}
         maxHeight={terminalRows - MODAL_TRANSCRIPT_PEEK}
         flexDirection="column" overflow="hidden" opaque={true}>
      <Box flexShrink={0}>
        <Text color="permission">{"▔".repeat(columns)}</Text>
      </Box>
      <Box flexDirection="column" paddingX={2} flexShrink={0} overflow="hidden">
        {modal}
      </Box>
    </Box>
  </ModalContext>
}
```

**Visual Layout:**
```
┌─────────────────────────────────┐
│ Messages (scrollable)           │
│                                 │
│                                 │
├─────────────────────────────────┤ ← ▔▔▔ divider (color="permission")
│  Modal Dialog Content           │ ← maxHeight = terminalRows - 2
│  paddingX=2                     │
│  columns - 4 width              │
│                                 │
└─────────────────────────────────┘
```

### 7.3 Dialog Component

**TypeScript Source:** `src/components/design-system/Dialog.tsx`

```tsx
interface DialogProps {
  title: React.ReactNode;          // Bold, colored title
  subtitle?: React.ReactNode;      // Dim subtitle
  children: React.ReactNode;       // Dialog body
  onCancel: () => void;            // Cancel handler (Esc/n)
  color?: keyof Theme;             // Default: "permission" (pink/red)
  hideInputGuide?: boolean;
  hideBorder?: boolean;
  inputGuide?: (exitState: ExitState) => React.ReactNode;  // Custom input guide
  isCancelActive?: boolean;        // Default: true (disable for text input fields)
}
```

**Structure:**
```
┌─ Pane(color) ────────────────────┐
│  Title (bold, color)             │
│  subtitle (dim)                  │
│                                  │
│  {children}                      │
│                                  │
│  [Enter] confirm  [Esc] cancel   │ ← input guide (dim, italic)
└──────────────────────────────────┘
```

**Key behaviors:**
- `useExitOnCtrlCDWithKeybindings()` — Ctrl+C/D exit guard
- `useKeybinding("confirm:no", onCancel)` — Esc/n to cancel
- `isCancelActive` — disables Esc/n/Ctrl+C/D while embedded text field is focused
- Wraps content in `<Pane color={color}>` unless `hideBorder`

### 7.4 Slash Commands and Their Dialogs

**TypeScript Source:** `src/commands.ts` (command registry), `src/commands/*/` (implementations)

Commands are categorized by `type`:
- **`'prompt'`**: Expands to text sent to model (skills)
- **`'local'`**: Returns text output
- **`'local-jsx'`**: Renders Ink UI dialog

#### Key `local-jsx` commands:

| Command | Dialog/UI | Description |
|---------|-----------|-------------|
| `/plugin` (`/plugins`, `/marketplace`) | `PluginSettings` → tabbed UI with DiscoverPlugins, ManagePlugins, ManageMarketplaces, PluginErrors | Full plugin management with search, install, configure |
| `/btw` | `BtwSideQuestion` — side question panel with scrolling markdown response | Ask a quick side question without interrupting main conversation |
| `/help` | `Help` dialog | Shows help and available commands |
| `/model` | Model picker dialog | Select AI model |
| `/theme` | Theme selection UI | Change terminal theme |
| `/color` | Color selection UI | Change agent color |
| `/keybindings` | Keybinding config dialog | Configure keyboard shortcuts |
| `/vim` | Toggle UI | Enable/disable vim mode |
| `/plan` | Plan mode dialog | Enter planning mode |
| `/permissions` | Permissions dialog | Manage permission settings |
| `/mcp` | MCP server management | Manage MCP servers |
| `/skills` | Skills browser | Browse available skills |
| `/memory` | Memory management | View/edit memory |
| `/config` | Configuration dialog | View/edit settings |
| `/session` | Session management | List/manage sessions |
| `/tasks` | BackgroundTasksDialog | View background tasks |
| `/mobile` | Mobile QR dialog | QR code for mobile pairing |
| `/stickers` | Sticker picker | Fun stickers |
| `/cost` | Text output | Show session cost |
| `/clear` (`/reset`, `/new`) | N/A (action) | Clear conversation |
| `/compact` | N/A (action) | Compact context |
| `/exit` | N/A (action) | Exit TUI |

#### `/plugin` Dialog Layout (most complex):

**TypeScript Source:** `src/commands/plugin/PluginSettings.tsx`

```
┌─ Modal (bottom-anchored)──────────────────────────┐
│ ▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔       │
│  Plugin Settings                                  │
│  ┌─ Tabs ──────────────────────────────────────┐  │
│  │ [Discover] [Installed] [Marketplaces] [...] │  │
│  ├─────────────────────────────────────────────┤  │
│  │                                             │  │
│  │  Tab content (varies by selection):         │  │
│  │  - DiscoverPlugins: SearchBox + list        │  │
│  │  - ManagePlugins: Installed plugin list     │  │
│  │  - ManageMarketplaces: Marketplace config   │  │
│  │  - PluginErrors: Error list + guidance      │  │
│  │                                             │  │
│  └─────────────────────────────────────────────┘  │
│  [Tab] switch  [Esc] close                        │
└───────────────────────────────────────────────────┘
```

#### `/btw` Dialog Layout (side question):

**TypeScript Source:** `src/commands/btw/btw.tsx`

The `/btw` command opens a **side question panel** that lets the user ask a quick question without interrupting the main conversation. It runs a **forked sub-agent** (`runSideQuestion`) with cached system prompts for efficiency.

```
┌─ /btw Panel ─────────────────────────────────────┐
│  paddingLeft=2, marginTop=1                      │
│                                                  │
│  /btw {your question here}                       │ ← /btw in warning color (bold), question in dimColor
│                                                  │
│  ┌─ ScrollBox (maxHeight = rows - 11) ────────┐  │
│  │  marginTop=1, marginLeft=2                 │  │
│  │                                            │  │
│  │  Loading state:                            │  │
│  │    [⠋] Answering... (warning color)        │  │
│  │                                            │  │
│  │  Success state:                            │  │
│  │    <Markdown>{response}</Markdown>         │  │
│  │                                            │  │
│  │  Error state:                              │  │
│  │    Error message (error color)             │  │
│  │                                            │  │
│  └────────────────────────────────────────────┘  │
│                                                  │
│  ↑/↓ to scroll · Space, Enter, or Esc to dismiss │ ← dimColor, only shown when response/error ready
└──────────────────────────────────────────────────┘
```

**Structure (React tree):**
```tsx
<Box flexDirection="column" paddingLeft={2} marginTop={1}
     tabIndex={0} autoFocus onKeyDown={handleKeyDown}>
  {/* Header: /btw label + question */}
  <Box>
    <Text color="warning" bold>/btw </Text>
    <Text dimColor>{question}</Text>
  </Box>

  {/* Scrollable content area */}
  <Box marginTop={1} marginLeft={2} maxHeight={maxContentHeight}>
    <ScrollBox ref={scrollRef} flexDirection="column" flexGrow={1}>
      {error ? <Text color="error">{error}</Text>
       : response ? <Markdown>{response}</Markdown>
       : <Box>
           <SpinnerGlyph frame={frame} messageColor="warning" />
           <Text color="warning">Answering...</Text>
         </Box>}
    </ScrollBox>
  </Box>

  {/* Footer help text (only when response/error is ready) */}
  {(response || error) && (
    <Box marginTop={1}>
      <Text dimColor>↑/↓ to scroll · Space, Enter, or Escape to dismiss</Text>
    </Box>
  )}
</Box>
```

**Key Layout Constants:**
```tsx
const CHROME_ROWS = 5;       // Inner chrome (header + scrollbox margins)
const OUTER_CHROME_ROWS = 6; // Outer chrome (panel margins + footer)
const SCROLL_LINES = 3;      // Lines scrolled per Up/Down keypress
```

**Content Height Calculation:**
```tsx
const maxContentHeight = Math.max(5, rows - CHROME_ROWS - OUTER_CHROME_ROWS);
// i.e., maxContentHeight = Math.max(5, terminalRows - 11)
```

**Key Behaviors:**
- **`immediate: true`** — Executes immediately without waiting for the model queue
- **Argument hint:** `<question>` — shown in typeahead
- **Forked sub-agent** — Uses `runSideQuestion()` with `CacheSafeParams` from the main conversation's last request (for prompt cache hit)
- **Spinner animation** — `useInterval` updates frame every 80ms while loading, stops on response/error
- **ScrollBox** — Uses `ScrollBox` with `ref` for imperative `scrollBy()` control
- **Usage tracking** — Increments `btwUseCount` in global config

**Keyboard Controls:**
| Key | Action |
|-----|--------|
| `Escape` / `Enter` / `Space` / `Ctrl+C` / `Ctrl+D` | Dismiss panel (`display: "skip"`) |
| `Up` / `Ctrl+P` | Scroll up 3 lines |
| `Down` / `Ctrl+N` | Scroll down 3 lines |

**Error Handling:**
- No question provided: Shows `Usage: /btw <your question>` as system message
- Fetch failure: Shows error message in red via `errorMessage(err)`
- Abort on unmount: `AbortController` cancels in-flight request on dismiss

#### Dialog Rendering Order (bottom section):

```tsx
<Box flexDirection="column" flexShrink={0} width="100%" maxHeight="50%">
  <SuggestionsOverlay />     {/* Slash command suggestions — bottom="100%" */}
  <DialogOverlay />          {/* Portaled dialogs — bottom="100%" */}
  <Box flexDirection="column" width="100%" flexGrow={1} overflowY="hidden">
    {bottom}                 {/* PromptInput — fills remaining space */}
  </Box>
</Box>
```

**Z-order (back to front):**
1. SuggestionsOverlay
2. DialogOverlay (paints over suggestions)
3. Modal (absolute bottom with ▔ divider — paints over everything)

### 7.5 Wizard Dialog Layout

**TypeScript Source:** `src/components/wizard/WizardDialogLayout.tsx`

Used for multi-step flows (agent creation, plugin configuration):

```tsx
<Dialog title={title} subtitle={subtitle} onCancel={goBack}
        color={color} hideInputGuide={true} isCancelActive={false}>
  {children}
</Dialog>
```

### 7.6 Other Dialog Types

| Dialog | Source | Purpose |
|--------|--------|---------|
| `ElicitationDialog` | `src/components/mcp/ElicitationDialog.tsx` | MCP server input requests |
| `TeamsDialog` | `src/components/teams/TeamsDialog.tsx` | Team/teammate management |
| `ShellDetailDialog` | `src/components/tasks/ShellDetailDialog.tsx` | Shell task details |
| `RemoteSessionDetailDialog` | `src/components/tasks/RemoteSessionDetailDialog.tsx` | Remote session details |
| `WorkflowDetailDialog` | `src/components/tasks/WorkflowDetailDialog.tsx` | Workflow task details |
| `PluginOptionsDialog` | `src/commands/plugin/PluginOptionsDialog.tsx` | Plugin configuration steps |
| `PermissionRequest` | `src/components/permissions/` | Tool use permissions |

---

## 8. Welcome Screen (Onboarding)

### 8.1 Overview

The welcome screen is shown **once** on first launch (before the main REPL starts) via the `showSetupDialog()` mechanism. It is a **full-screen modal dialog** that blocks the REPL until the user completes or skips onboarding.

**TypeScript Source:** `src/components/LogoV2/WelcomeV2.tsx`, `src/components/Onboarding.tsx`

**Trigger:** `src/interactiveHelpers.tsx` → `showSetupScreens()` checks `config.hasCompletedOnboarding`

```typescript
if (!config.theme || !config.hasCompletedOnboarding) {
    await showSetupDialog(root, done =>
        <Onboarding onDone={() => { completeOnboarding(); void done(); }} />
    );
}
```

### 8.2 WelcomeV2 ASCII Art

**TypeScript Source:** `src/components/LogoV2/WelcomeV2.tsx`

The welcome screen renders a **58-character wide** ASCII art logo with theme-aware variations:

```tsx
const WELCOME_V2_WIDTH = 58;
```

**Three rendering paths:**

| Condition | Component | Notes |
|-----------|-----------|-------|
| `env.terminal === "Apple_Terminal"` | `AppleTerminalWelcomeV2` | Special rendering for macOS Terminal.app |
| Light themes (`light`, `light-daltonized`, `light-ansi`) | Standard WelcomeV2 (light variant) | Uses block characters (`░`, `▒`, `▓`) for shading |
| Dark themes (default) | Standard WelcomeV2 (dark variant) | Uses stars (`*`) for sparkle, block characters for shading |

#### Dark Theme Welcome Screen (default):

```
Welcome to Claude Code v{version}
……………………………………………………………………………………………………………………
                                                                  
     *                                       ██████▓▓░     
                                 *         ███▓░     ░░   
            ░░░░░░                        ███▓░           
    ░░░   ░░░░░░░░░░                      ███▓░           
   ░░░░░░░░░░░░░░░░    *                ██▓░░      ▓   
                                             ░▓▓███▓▓░    
 *                                 ░░░░                   
                                 ░░░░░░░░                 
                               ░░░░░░░░░░░░░░░           
      █████████                                          * 
      ██▄█████▄██                        *               
      █████████     *                                     
……………………………█ █   █ █………………………………………………………………………………
```

**Color elements:**
- `"Welcome to Claude Code"` — `color="claude"` (brand color)
- `v{version}` — `dimColor`
- Body blocks — `color="clawd_body"` with `backgroundColor="clawd_background"` on center
- Shading — `░`, `▒`, `▓` block characters
- Bottom signature — `█ █   █ █` in `color="clawd_body"` surrounded by `…`

#### Light Theme Welcome Screen:

```
Welcome to Claude Code v{version}
……………………………………………………………………………………………………………………
                                                                  
                                                                  
                                                                  
            ░░░░░░                                              
    ░░░   ░░░░░░░░░░                                            
   ░░░░░░░░░░░░░░░░                                             
                                                                  
                           ░░░░                     ██    
                         ░░░░░░░░░           ██▒▒██  
                                            ▒▒      ██   ▒
      █████████                          ▒▒░░▒▒      ▒ ▒▒
      ██▄█████▄██                        ▒▒         ▒▒ 
      █████████                           ░          ▒   
……………………………█ █   █ █……………………………………░……………………▒…………
```

**Key difference:** Light theme uses inverted colors — `color="clawd_body"` with `backgroundColor="clawd_body"` swap, and space-filled blocks instead of solid blocks.

### 8.3 Onboarding Flow

**TypeScript Source:** `src/components/Onboarding.tsx`

The `Onboarding` component wraps `WelcomeV2` with a **multi-step wizard** below it:

```tsx
<Box flexDirection="column">
  <WelcomeV2 />
  <Box flexDirection="column" marginTop={1}>
    {currentStep?.component}
    {exitState.pending && (
      <Box padding={1}>
        <Text dimColor>Press {exitState.keyName} again to exit</Text>
      </Box>
    )}
  </Box>
</Box>
```

#### Onboarding Steps (in order):

| Step ID | Component | Description |
|---------|-----------|-------------|
| `preflight` | `PreflightStep` | System checks (only if OAuth enabled) |
| `theme` | `ThemePicker` | Select terminal theme (dark/light/etc.) |
| `api-key` | `ApproveApiKey` | Approve custom API key (if configured) |
| `oauth` | `ConsoleOAuthFlow` | OAuth authentication (wrapped in `SkippableStep`) |
| `security` | Security step | Security settings and permissions |
| `terminal-setup` | Terminal setup | Enable terminal features (newlines, visual bell) |

**Navigation:**
- `confirm:yes` keybinding → advance to next step (context: "Confirmation")
- `confirm:no` keybinding → skip terminal-setup step
- Final step → calls `onDone()` which sets `hasCompletedOnboarding = true`

### 8.4 Setup Dialog Rendering

**TypeScript Source:** `src/interactiveHelpers.tsx`

```typescript
export function showDialog<T = void>(
    root: Root,
    renderer: (done: (result: T) => void) => React.ReactNode,
): Promise<T> {
    return new Promise<T>(resolve => {
        const done = (result: T): void => void resolve(result);
        root.render(renderer(done));
    });
}

export function showSetupDialog<T = void>(
    root: Root,
    renderer: (done: (result: T) => void) => React.ReactNode,
    options?: { onChangeAppState?: typeof onChangeAppState },
): Promise<T> {
    return showDialog<T>(root, done =>
        <AppStateProvider onChangeAppState={options?.onChangeAppState}>
            <KeybindingSetup>{renderer(done)}</KeybindingSetup>
        </AppStateProvider>
    );
}
```

**Key:** The dialog is rendered as a **standalone Ink render** (not inside the REPL's FullscreenLayout). It blocks via `await showSetupDialog(...)` — the Promise resolves when `done()` is called.

### 8.5 Complete Setup Screen Sequence

```
1. showSetupScreens() called before REPL starts
   │
   ├─ Check: hasCompletedOnboarding? ── Yes ──→ Skip to TrustDialog
   │  │
   │  No
   │  │
   │  ├─ Render Onboarding (WelcomeV2 + step wizard)
   │  │  └─ await showSetupDialog(...) → Promise blocks
   │  │     └─ User completes steps → onDone() → completeOnboarding() → done()
   │  │        └─ Promise resolves → continues
   │  │
   │  └─ TrustDialog (if CWD not trusted)
   │     └─ await showSetupDialog(...)
   │
2. Main REPL renders (FullscreenLayout with messages + prompt)
```

### 8.6 Welcome Screen in Other Contexts

The `WelcomeV2` component is also rendered standalone in:
- **CLI `--help` handler:** `src/cli/handlers/util.tsx` — shows welcome + help text
- **Apple Terminal special case:** `AppleTerminalWelcomeV2` uses different character set for macOS Terminal.app compatibility

---

## 9. Unseen Divider & Pill

### TypeScript: `useUnseenDivider` hook

**Logic:**
1. On first scroll-away from bottom:
   - Snapshot `dividerY = scrollHeight`
   - Set `dividerIndex = messageCount`
2. Count assistant turns from dividerIndex to end
3. Show pill if `scrollTop + pendingDelta + viewportHeight < dividerY`

### Rust: `src/tui/widgets/fullscreen_layout.rs`

```rust
pub fn compute_unseen_divider(
    messages: &[Message],
    divider_index: Option<usize>,
) -> Option<UnseenDivider> {
    // Skip progress messages
    // Count assistant turns
    // Return { firstUnseenUuid, count }
}

pub struct NewMessagesPillWidget {
    pub count: usize,  // 0 = "Jump to bottom", >0 = "N new message(s)"
}
```

**Pill Text:**
- `count == 0`: "Jump to bottom ↓"
- `count > 0`: "{count} new message(s) ↓"

---

## 9. Layout Calculations

### Content Height Calculation

```rust
let mut total_content_lines = 0u16;
for msg in messages.iter() {
    let lc = registry.find_renderer(msg)
        .map(|r| r.line_count(msg, width))
        .unwrap_or_else(|| msg.content.lines().count().max(1));
    total_content_lines += lc as u16;
}
if is_loading && !app.streaming_text.is_empty() {
    total_content_lines += app.streaming_text.lines().count().max(1) as u16;
}
if is_loading {
    total_content_lines += 1; // Spinner line
}
```

### Layout Decision

```rust
let gap = 1u16;
let min_bottom_h = 3u16;

let (scroll_h, bottom_y, bottom_h) = 
    if total_content_lines + gap + min_bottom_h <= max_combined_h {
        // Content fits on screen
        (total_content_lines + gap, sh, bh)
    } else {
        // Content overflows
        (max_combined_h - min_bottom_h, sh, min_bottom_h)
    };
```

---

## 10. Key State Variables

### App State: `src/tui/app.rs`

```rust
pub struct App {
    // Messages
    pub messages: Vec<Message>,
    pub streaming_text: String,
    pub streaming_thinking: Option<StreamingThinking>,
    
    // Scroll
    pub scroll_offset: usize,
    pub sticky_scroll: bool,
    pub prev_message_count: usize,
    
    // Loading
    pub is_loading: bool,
    pub loading_start_time: Option<Instant>,
    pub response_length: usize,
    pub last_response_length: usize,
    
    // Spinner
    pub spinner_frame: usize,
    pub spinner_animation_time: u64,
    pub spinner_message: Option<String>,
    pub spinner_verb: String,
    pub stream_mode: String,
    
    // Layout
    pub scroll_chrome: ScrollChromeContext,
    pub scroll_handle: ScrollBoxHandle,
    pub unseen_divider: Option<UnseenDivider>,
}
```

---

## 11. File Structure

```
src/tui/
├── app.rs                          # Main App state + event handling
├── repl.rs                         # REPL state + render entry
├── repl_main_render.rs            # Main layout + scroll logic
├── repl_query.rs                  # Query handling + message management
├── spinner.rs                     # Spinner animation
├── spinner_glyph.rs               # Spinner characters
└── widgets/
    ├── fullscreen_layout.rs        # FullscreenLayout component
    ├── scroll_box.rs              # ScrollBox widget
    ├── message_renderers/         # Message type renderers
    ├── dialog_system.rs           # Dialog management
    ├── assistant_thinking.rs      # Thinking message widget
    └── ...
```

---

## 12. Common Pitfalls

### ❌ DON'T:
- Bundle spinner with input box (they're in different layout sections)
- Use `app.messages.pop()` to remove placeholder (use `remove_placeholder()`)
- Update `last_response_length` during streaming (only on message complete)
- Forget to set `sticky_scroll = false` on user manual scroll
- Render modal without ▔ divider line
- Skip overscan in virtual scroll (causes blank space)

### ✅ DO:
- Keep spinner in scrollable area
- Use `ScrollBoxHandle.subscribe()` for scroll listeners
- Calculate content height BEFORE layout decision
- Clear `streaming_text` on `ContentBlockStart`
- Show thinking AFTER streaming ends (30s timeout)
- Use `Rc<RefCell<>>` for shared scroll state (not raw pointers)

---

## 13. TypeScript ↔ Rust Mapping

| TypeScript | Rust | Location |
|------------|------|----------|
| `useVirtualScroll` | `use_virtual_scroll()` | `src/hooks/` |
| `ScrollBox` | `ScrollBox` widget | `src/tui/widgets/scroll_box.rs` |
| `FullscreenLayout` | `render_fullscreen_layout()` | `src/tui/widgets/fullscreen_layout.rs` |
| `Messages` | Message renderers | `src/tui/widgets/message_renderers/` |
| `SpinnerWithVerb` | Spinner + shimmer | `src/tui/repl_main_render.rs` |
| `PromptInput` | Prompt render | `src/tui/repl_main_render.rs` |
| `useUnseenDivider` | `compute_unseen_divider()` | `src/tui/widgets/fullscreen_layout.rs` |
| `NewMessagesPill` | `NewMessagesPillWidget` | `src/tui/widgets/fullscreen_layout.rs` |
| `StickyPromptHeader` | `StickyPromptHeaderWidget` | `src/tui/widgets/fullscreen_layout.rs` |

---

## 14. Testing

```bash
# Build
cargo build

# Run tests
cargo test

# Expected: 618+ passed, 1 pre-existing failure (model label)
```

---

## 15. References

- TypeScript source: `~/claudecode/openclaudecode/`
- Main REPL: `src/screens/REPL.tsx`
- FullscreenLayout: `src/components/FullscreenLayout.tsx`
- ScrollBox: `src/ink/components/ScrollBox.tsx`
- VirtualScroll: `src/hooks/useVirtualScroll.ts`
- Spinner: `src/components/Spinner.tsx` + `SpinnerAnimationRow.tsx`

---

*Last updated: 2026-04-13*
*Verified against TypeScript commit: main branch*
