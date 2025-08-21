# nXCC Documentation

Contributing to the nXCC documentation site built with [Astro Starlight](https://starlight.astro.build/).

## Getting Started

### Prerequisites

- Node.js 18+
- pnpm

### Setup

```bash
# Install dependencies
pnpm install

# Start development server
pnpm dev

# Build static site
pnpm build

# Preview production build
pnpm preview
```

## Documentation Structure

The documentation lives in `src/content/docs/docs/`:

```
src/content/docs/docs/
├── guides/              # Getting started guides
└── reference/           # API and reference documentation
    ├── cli.md
    ├── core-concepts.md
    ├── event-triggers.md
    ├── identities-and-policies.md
    ├── performance.md
    ├── running-a-node.md
    ├── worker-manifest.md
    └── worker-runtime.md
```

## Writing Documentation

### Markdown Basics

- Use Markdown (`.md`) for most content
- Use MDX (`.mdx`) only when you need Vue components
- Place images in `src/assets/` and reference with relative paths

### Starlight Features

- **Frontmatter**: Add title, description, and sidebar configuration
- **Code blocks**: Syntax highlighting with language tags
- **Callouts**: Use `:::note`, `:::tip`, `:::warning`, `:::danger`
- **Tabs**: Group related content with tab components

Example frontmatter:

```yaml
---
title: Page Title
description: Brief description for SEO
sidebar:
  order: 3
---
```

### Content Guidelines

- **Target audience**: Developers integrating with nXCC
- **Technical depth**: Assume familiarity with blockchain/Web3 concepts
- **Code examples**: Always include working, testable code
- **Links**: Use relative links for internal pages

## Site Configuration

Key files for site-wide changes:

- `astro.config.mjs`: Starlight and build configuration
- `src/content.config.ts`: Content validation schemas
- `package.json`: Build scripts and dependencies

## Local Development

```bash
# Start dev server (hot reload enabled)
pnpm dev

# Check for broken links
pnpm build 2>&1 | grep -i "warning\|error"

# Format all files
pnpm format
```

The site will be available at `http://localhost:4321` with hot reload for immediate feedback.

## Contributing Process

1. Create or edit content in `src/content/docs/docs/`
2. Test locally with `pnpm dev`
3. Build and verify with `pnpm build && pnpm preview`
4. Submit pull request

For Starlight-specific features and syntax, see the [Starlight documentation](https://starlight.astro.build/guides/authoring-content/).
