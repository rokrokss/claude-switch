# apps/landing

Marketing landing page for claude-switch. Static Astro + Tailwind v4.

## Scripts

- `pnpm --filter landing dev` — local dev server on http://localhost:4321
- `pnpm --filter landing build` — emit static site to `dist/`
- `pnpm --filter landing preview` — preview the built site
- `pnpm --filter landing format` — format files with Biome

## Deploying

Served by GitHub Pages at https://claude-switch.shizue.net. `.github/workflows/pages.yml` builds the site and deploys it on every push to `main` that touches the landing page, the design tokens or the desktop app's version, and can be run by hand from the Actions tab.

### One-time setup

1. Repository Settings → Pages → **Source**: GitHub Actions.
2. **Custom domain**: `claude-switch.shizue.net` (also in `public/CNAME`), then **Enforce HTTPS** once the certificate is issued.
3. DNS: a `CNAME` record for `claude-switch.shizue.net` pointing at `rokrokss.github.io`.

Analytics stay off unless the workflow is given `PUBLIC_POSTHOG_KEY` (and optionally `PUBLIC_POSTHOG_HOST`, default `https://eu.i.posthog.com`) as repository variables.
