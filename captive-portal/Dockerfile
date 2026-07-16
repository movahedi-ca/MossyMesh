# Multi-stage image: build portal + frontend, serve with hardened nginx.
# client_max_body_size 150M is set in nginx.conf.

# ---------- portal build ----------
FROM node:22-alpine AS portal-build
WORKDIR /src/portal
COPY captive-portal/package.json captive-portal/package-lock.json ./
RUN npm ci
COPY captive-portal/ ./
RUN npm run build

# ---------- frontend build ----------
FROM node:22-alpine AS frontend-build
WORKDIR /src/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
# Base path /app/ so assets resolve when served under /app/ by nginx
RUN npx tsc -b && npx vite build --base=/app/

# ---------- runtime ----------
FROM nginx:1.27-alpine
LABEL org.opencontainers.image.title="MessyMash Captive Portal"
LABEL org.opencontainers.image.description="Offline mesh captive portal + frontend static host"

COPY captive-portal/nginx.conf /etc/nginx/nginx.conf

# Portal landing at document root
COPY --from=portal-build /src/portal/dist /usr/share/nginx/html
# Chess PWA under /app/
COPY --from=frontend-build /src/frontend/dist /usr/share/nginx/html/app

RUN printf '%s\n' \
  '<!DOCTYPE html><html><body><h1>MessyMash node error</h1></body></html>' \
  > /usr/share/nginx/html/50x.html

EXPOSE 80
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD wget -qO- http://127.0.0.1/healthz || exit 1

CMD ["nginx", "-g", "daemon off;"]
