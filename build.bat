rem cargo build --release
rem upx target\release\rs_music_server.exe

for /f %%i in ('powershell -NoProfile -Command "(Get-Date).ToUniversalTime().ToString(\"yyyy-MM-ddTHH:mm:ssZ\")"') do set BUILD_TIMESTAMP=%%i

docker build --build-arg BUILD_TIMESTAMP=%BUILD_TIMESTAMP% . -t ghcr.io/chrisheib/rs_music_server
docker push ghcr.io/chrisheib/rs_music_server
