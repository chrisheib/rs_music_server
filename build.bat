rem cargo build --release
rem upx target\release\rs_music_server.exe

docker build . -t ghcr.io/chrisheib/rs_music_server
docker push ghcr.io/chrisheib/rs_music_server
