FROM node:26.13.0-alpine3.24 AS build

WORKDIR /app

# Node 25+ no longer bundles corepack, so install the pnpm pinned in package.json.
# npm 11 skips dependency install scripts by default; pnpm's only swaps its bins
# for the native binary, so allow it to avoid the warning and the Node.js hop.
RUN npm install -g --allow-scripts=pnpm pnpm@12.16.1

COPY package.json pnpm-lock.yaml pnpm-workspace.yaml /app/
RUN pnpm install --frozen-lockfile

COPY . /app
RUN pnpm build

FROM nginx:1.27-alpine

# The image's entrypoint renders templates into conf.d. With LOCAL_RESOLVERS set it
# also exports the container's DNS servers as NGINX_LOCAL_RESOLVERS for the template.
ENV NGINX_ENTRYPOINT_LOCAL_RESOLVERS=1
COPY nginx.conf /etc/nginx/templates/default.conf.template
COPY --from=build /app/dist /usr/share/nginx/html

EXPOSE 80
