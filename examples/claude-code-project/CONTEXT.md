# Project Context

This is a web application built with TypeScript and React.

## Architecture

- **Frontend**: React with TypeScript, using Vite for bundling.
- **Backend**: Node.js with Express, PostgreSQL database.
- **Testing**: Vitest for unit tests, Playwright for E2E tests.

## Conventions

- Use functional components with hooks (no class components).
- All API endpoints return JSON and follow REST conventions.
- Database queries use parameterized statements (never string interpolation).
- Error handling uses a centralized error middleware.

## Directory Structure

- `src/` -- application source code
  - `src/components/` -- React components
  - `src/api/` -- API route handlers
  - `src/db/` -- database models and migrations
- `tests/` -- test files mirroring the src structure
- `scripts/` -- build and deployment scripts

## Important Notes

- Never commit `.env` files. Use `.env.example` as a template.
- Run `npm run typecheck` before committing to catch type errors.
- The CI pipeline runs on every pull request and must pass before merging.
