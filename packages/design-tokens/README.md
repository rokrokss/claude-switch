# @claude-switch/design-tokens

Single source of truth for colour, radius, shadow, typography tokens. CSS-only.

## Usage

```css
@import "@claude-switch/design-tokens/tokens.css";
```

Tailwind v4 reads the `@theme { … }` block to generate utility classes (`bg-cream`, `text-ink`, `shadow-window`, etc.). Consumed by `apps/claude-switch/` and `apps/landing/`.
