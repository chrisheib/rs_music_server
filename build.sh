#!/usr/bin/env bash
sudo docker build . -t ghcr.io/chrisheib/rs_music_server && \
sudo docker push ghcr.io/chrisheib/rs_music_server && \
ssh minischiff "cd ~/Downloads/docker/music_srv && sudo docker compose pull && sudo docker compose up -d"
