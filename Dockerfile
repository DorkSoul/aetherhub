# syntax=docker/dockerfile:1
# The rules engine: a Rust workspace compiled to a native Node addon.
FROM rust:1-slim-bookworm AS engine
WORKDIR /src/engine
COPY engine/Cargo.toml engine/Cargo.lock ./
COPY engine/crates ./crates
RUN cargo build --release --locked -p mtg-node && cp target/release/libmtg_node.so /mtg_node.so

FROM node:24-slim AS build
WORKDIR /app
COPY package.json package-lock.json ./
COPY packages/shared/package.json packages/shared/
COPY packages/rules/package.json packages/rules/
COPY packages/server/package.json packages/server/
COPY packages/client/package.json packages/client/
RUN npm ci --no-audit --no-fund
COPY tsconfig.base.json ./
COPY packages ./packages
RUN npm run build && npm prune --omit=dev --no-audit --no-fund

FROM node:24-slim
ENV NODE_ENV=production PORT=8080 DATA_DIR=/data RULES_NATIVE=/app/native/libmtg_node.so RULES_CARDS=/app/cards
WORKDIR /app
COPY --from=build /app/package.json ./
COPY --from=build /app/node_modules ./node_modules
COPY --from=build /app/packages/server/package.json ./packages/server/
COPY --from=build /app/packages/server/dist ./packages/server/dist
COPY --from=build /app/packages/client/dist ./packages/client/dist
COPY --from=engine /mtg_node.so ./native/libmtg_node.so
COPY cards ./cards
COPY decks/precons.json ./decks/precons.json
RUN mkdir -p /data && chown node:node /data
USER node
VOLUME /data
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s \
  CMD node -e "fetch('http://127.0.0.1:'+(process.env.PORT||8080)+(process.env.BASE_PATH||'/').replace(/\/?$/,'/')+'api/health').then(r=>process.exit(r.ok?0:1)).catch(()=>process.exit(1))"
CMD ["node", "packages/server/dist/index.js"]
