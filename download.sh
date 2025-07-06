#!/usr/bin/env bash
read -e -p "Enter YouTube URL: " URL

vid=$(sed -E 's#.*(v=|youtu\.be/)([^&?]+).*#\2##p' <<< "$URL")
echo "ID is: $vid"

# Download, extract audio → mp3, and print the final file‐path only
FILE=$(yt-dlp -q \
  --no-warnings \
  --extract-audio \
  --audio-format mp3 \
  -o '~/Downloads/yt-dlp/%(title)s.%(ext)s' \
  --print after_move:filepath \
  "https://www.youtube.com/watch?v=$vid")

echo "Downloaded to: $FILE"
mp3gain -r -d 10 -k "$FILE"

# UPLOAD_URL="http://localhost:3001/upload"
UPLOAD_URL="http://192.168.2.250:3001/upload"
curl -v "$UPLOAD_URL" -F "file=@$FILE"
#   -H "Authorization: Bearer $API_TOKEN" \
#   -F "title=$(basename "$FILE" .mp3)"