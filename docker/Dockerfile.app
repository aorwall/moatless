# moatless-frontend: Static frontend nginx image
# Expects pre-built frontend assets in apps/frontend/dist
# (build-app.sh runs the project build before Docker)
#
# Published as: aorwall/moatless-frontend:latest

FROM nginx:1.31-alpine

# Pull the latest patched Alpine packages on top of the base image. Bump the
# refresh date to invalidate this layer and force `apk upgrade` to re-pull the
# newest security fixes for a CVE rebuild (mirrors the apt-get upgrade cache-bust
# in Dockerfile.sandbox-base).
# 2026-07-13: clear curl/libcurl CVEs in moatless-frontend (SP-2236) —
# CVE-2026-9079 (CVSS 9.8), CVE-2026-8926 (9.1), CVE-2026-9545 (7.5),
# affecting curl 8.12.1-r1 shipped in the previous nginx:1.27-alpine base.
# 2026-09-07: pull patched libssl3/libcrypto3 > 3.5.7-r0 (CVE-2026-63073 CVSS
# 9.8, CVE-2026-75803 9.1) and libexpat >= 2.8.4-r0 (CVE-2026-76641 8.7), OP-760.
# 2026-09-14: pull patched curl/libcurl >= 8.22.0-r0 (CVE-2026-19931 CVSS 9.8,
# CVE-2026-18924 9.1, CVE-2026-82209 8.2, CVE-2026-80229, -80230, -80231,
# -80255, -82208 7.5, CVE-2026-13608 7.4) and libexpat >= 2.8.4-r0
# (CVE-2026-76956 7.5, CVE-2026-76957 7.8), OP-799.
ARG APK_SECURITY_REFRESH=2026-09-14
RUN echo "OS security refresh: ${APK_SECURITY_REFRESH}" \
    && apk upgrade --no-cache

# Copy pre-built frontend assets
COPY apps/frontend/dist /usr/share/nginx/html

# Copy nginx configuration as an envsubst template. The base image entrypoint
# renders /etc/nginx/templates/*.template into /etc/nginx/conf.d/ at startup,
# substituting ${CSP_FRAME_SRC} (nginx's own $vars are left untouched — the
# entrypoint only substitutes names present in the environment).
COPY docker/nginx.conf /etc/nginx/templates/default.conf.template

# Remove the base image's stock server config. The rendered template is the only
# config; if the entrypoint cannot write it (conf.d not writable under a non-root
# read-only rootfs), nginx must fail its probe loudly rather than silently serve
# this stock block, which returns 200 for `/` but 404s every SPA deep link.
RUN rm -f /etc/nginx/conf.d/default.conf

# CSP frame-src value. Defaults to the external https preview URLs; local http
# clusters override it (e.g. to add http://*.preview.moatless.localhost).
ENV CSP_FRAME_SRC="https:"

EXPOSE 80

CMD ["nginx", "-g", "daemon off;"]
