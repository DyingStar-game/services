# Auth adapter for the text-chat MQTT broker (validates Keycloak JWTs for go-auth).
FROM node:20-alpine

WORKDIR /app

# Install production dependencies first (better layer caching).
COPY package.json ./
RUN npm install --omit=dev

COPY src ./src

ENV NODE_ENV=production
ENV PORT=3000
EXPOSE 3000

CMD ["node", "src/index.js"]
