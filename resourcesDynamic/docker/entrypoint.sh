#!/bin/sh
set -e

echo "🔄 Running database migrations..."
npm run db:migrate

echo "📥 Importing system data..."
node dist/scripts/import/import-system.mjs

echo "🚀 Starting application..."
exec node dist/index.mjs
