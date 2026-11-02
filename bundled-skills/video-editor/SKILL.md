---
name: video-editor
description: Thin video workflow. Use when making, cutting or reviewing a video, Short or reel - start from the project's own pipeline, pick one tool per job, check the finished file, and leave publishing to a person.
---

# Video editor

A short routine: use what the project has, pick one tool, check the file. It is
deliberately thin. The tools below are pinned in the catalog, not bundled here.

## 1. Start from what exists

- If the project has a video pipeline, extend it. Do not start a second one.
- Decide the one idea before touching a timeline. A new tool does not fix a weak script.

## 2. Pick one tool per job

| Job | Use |
| --- | --- |
| Chart or data scene | `remotion-video`, with `video-shotcraft` recipes |
| Worked math | `manim-video` |
| Cuts, loudness, captions on a file | The project's ffmpeg first, then `ffmpeg-skill` |
| Talking-head edit | `ghost-editor`, own recordings only; a person approves cuts |
| Narration | `kokoro` |
| Sound effects | `earshot` |
| Titles, hooks, metadata | `claude-youtube`, `yt-viral`, `ig-reel` |

`skill-catalog` installs a missing skill and lists each tool's conditions.

## 3. Check the file

```
ffprobe -v error -show_entries stream=codec_type,width,height:format=duration -of json out.mp4
ffmpeg -hide_banner -i out.mp4 -af ebur128 -f null - 2>&1 | tail -12
```

- The file has one video and one audio stream, at the size asked for: 1080x1920 for a
  Short.
- Integrated loudness is near -14 LUFS.

Then watch it and ask:

- Does the first two seconds say why to keep watching?
- Do captions stay inside the safe area and stay readable on a phone?
- Does every number on screen match the script and its source?
- Is there one idea, with nothing stretched, frozen or out of sync?

Passing means not broken, not good. A person still judges it.

## 4. Rules

- Use only commercially licensed assets, and log each one in the project's credits file.
- Never copy another creator's transcript or clone their video.
- Publishing is a protected action. Build a dry run and hand it over; a person approves
  any upload.
