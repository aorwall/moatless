# moatless-vibe: Static frontend nginx image
# Expects pre-built frontend assets in apps/frontend/dist
# (build-app.sh runs the project build before Docker)
#
# Published as: aorwall/moatless-vibe:latest

FROM nginx:1.27-alpine

# Copy pre-built frontend assets
COPY apps/frontend/dist /usr/share/nginx/html

# Copy nginx configuration
COPY docker/nginx.conf /etc/nginx/conf.d/default.conf

EXPOSE 80

CMD ["nginx", "-g", "daemon off;"]
