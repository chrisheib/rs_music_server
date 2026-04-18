#!/usr/bin/env bash
build_timestamp="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

sudo docker build --build-arg BUILD_TIMESTAMP="$build_timestamp" . -t ghcr.io/chrisheib/rs_music_server && \
sudo docker push ghcr.io/chrisheib/rs_music_server && \
ssh minischiff "cd ~/Downloads/docker/music_srv && sudo docker compose pull && sudo docker compose up -d"
