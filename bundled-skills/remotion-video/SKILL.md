---
name: remotion-video
description: React-based programmatic video creation with Remotion. Activate for video generation, animation, programmatic video, React video, motion graphics tasks.
user-invocable: true
---

# Remotion Video Engineering

## When to Activate
- User mentions: remotion, programmatic video, react video, animated video, motion graphics, video template
- Building or modifying video generation pipelines
- Creating reusable video templates with dynamic data

## Why Remotion Over FFmpeg
| Factor | FFmpeg | Remotion |
|--------|--------|---------|
| Visual complexity | Filter chain hell | React components |
| Text animation | Static overlays | Kinetic typography, easing |
| Data visualization | Manual plotting | D3/Recharts in video |
| Templating | Shell variable substitution | Props-driven React components |
| Preview | Re-render entire video | Hot reload in browser |
| Reusability | Copy-paste filter chains | Import/export components |
| Debugging | FFmpeg log parsing | React DevTools + browser |

## Project Setup
```bash
npx create-video@latest my-video --template blank
cd my-video && npm start  # Opens preview at localhost:3000
```

**Structure:**
```
src/
├── Root.tsx              # Composition registry
├── Video.tsx             # Main composition
├── components/
│   ├── TitleCard.tsx     # Reusable title animation
│   ├── LowerThird.tsx    # Name/topic overlay
│   ├── DataChart.tsx     # Animated chart component
│   └── Transition.tsx    # Scene transitions
├── data/                 # Dynamic content (JSON, API)
└── assets/               # Fonts, images, audio
```

## Core Patterns

### 1. Composition (Video Definition)
```tsx
export const MyVideo: React.FC<{title: string; data: any[]}> = ({title, data}) => {
  return (
    <AbsoluteFill style={{backgroundColor: '#0A0E27'}}>
      <Sequence from={0} durationInFrames={120}>
        <TitleCard text={title} />
      </Sequence>
      <Sequence from={120} durationInFrames={300}>
        <MainContent data={data} />
      </Sequence>
      <Sequence from={420} durationInFrames={90}>
        <OutroCard />
      </Sequence>
    </AbsoluteFill>
  );
};
```

### 2. Animation (spring + interpolate)
```tsx
const frame = useCurrentFrame();
const opacity = interpolate(frame, [0, 30], [0, 1], {extrapolateRight: 'clamp'});
const scale = spring({frame, fps: 30, config: {damping: 12, stiffness: 200}});
```

### 3. Data-Driven Video
```tsx
// Feed JSON data → video renders dynamically
const {data} = useVideoConfig();
// Each data point becomes a scene, chart, or overlay
```

### 4. Audio Sync
```tsx
<Audio src={voiceoverUrl} />
<Sequence from={audioTimestamp * fps}>
  <VisualForThisSection />
</Sequence>
```

## Migration Path (FFmpeg → Remotion)
1. Keep FFmpeg for encoding (Remotion uses it under the hood)
2. Replace `video_assembler.py` overlay logic with React components
3. Convert text overlays → `<TitleCard>`, `<LowerThird>` components
4. Convert footage concatenation → `<Sequence>` chains with transitions
5. Convert BGM mixing → `<Audio>` component with volume control
6. Render: `npx remotion render src/index.ts MyVideo output.mp4`

## Rendering
```bash
# Local render
npx remotion render src/index.ts MyVideo output.mp4 --codec h264

# Lambda render (parallel, fast)
npx remotion lambda render --function-name my-func --composition MyVideo

# Programmatic (from Python pipeline)
subprocess.run(["npx", "remotion", "render", "src/index.ts", "MyVideo",
    f"--props", json.dumps(video_data), output_path])
```

## Guardrails
- ALWAYS preview before rendering full video (hot reload at localhost:3000)
- NEVER hardcode text — use props for all dynamic content
- ALWAYS use `spring()` for natural motion (not linear interpolation)
- Design mobile-first: text must be legible at 360p (YouTube Shorts)
- Keep compositions under 60 seconds for Shorts, 8-20 min for long-form
