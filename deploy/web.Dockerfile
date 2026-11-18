# syntax=docker/dockerfile:1
# The frontend, built for this server: the template at FRONTEND_REF, built with
# the mock API off and served by its own deploy/nginx.conf with the /api/
# proxy switched on (it forwards to the ax-backend service).

FROM node:24-bookworm-slim AS build
ARG FRONTEND_REPO=https://github.com/Dowon-Kim7949/ax-request-portal.git
ARG FRONTEND_REF
RUN apt-get update \
 && apt-get install -y --no-install-recommends git ca-certificates \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /app
RUN test -n "$FRONTEND_REF" \
 && git init -q . \
 && git remote add origin "$FRONTEND_REPO" \
 && git fetch -q --depth 1 origin "$FRONTEND_REF" \
 && git checkout -q FETCH_HEAD
RUN npm ci
RUN VITE_API_MOCK=false VITE_API_BASE_URL=/api VITE_BUILD_ID="$FRONTEND_REF" npm run build
# Uncomment the template's same-origin backend block.
RUN sed -i -E '/^    # location \/api\/ \{/,/^    # \}/ s/^    # /    /' deploy/nginx.conf \
 && grep -q '^    location /api/ {' deploy/nginx.conf

FROM nginx:stable-alpine
COPY --from=build /app/dist /usr/share/nginx/html
COPY --from=build /app/deploy/nginx.conf /etc/nginx/conf.d/default.conf
