/**
 * Serves the service's OpenAPI document (`openapi.yaml`) from the project root.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import type { Request, Response } from 'express';

// src/lib/openapi.ts and dist/lib/openapi.js both resolve to the project root's openapi.yaml.
const specPath = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..', 'openapi.yaml');

let cached: string | null | undefined;

/**
 * Returns the OpenAPI YAML document, or null when the file cannot be read.
 * @returns Document contents (cached after the first read).
 */
export function openapiDocument(): string | null {
  if (cached === undefined) {
    try {
      cached = readFileSync(specPath, 'utf8');
    } catch {
      cached = null;
    }
  }
  return cached;
}

/**
 * Express handler responding with the OpenAPI document as `application/yaml`.
 * @param _req - Express request.
 * @param res - Express response.
 */
export function serveOpenapi(_req: Request, res: Response): void {
  const spec = openapiDocument();
  if (!spec) {
    res.status(404).json({ error: 'NOT_FOUND', message: 'OpenAPI document not available', status: 404 });
    return;
  }
  res.type('application/yaml').send(spec);
}
