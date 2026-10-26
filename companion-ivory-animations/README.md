# Ivory robot — animation sequences, v1

Created with the built-in ImageGen on October 21, 2026, starting from the chosen ivory concept. Concept material for developing the animations, not sprites already validated for production.

Each sheet contains six frames: first row left to right (1–3), then second row (4–6). Compared with the initial proposal, hold poses were added to get a uniform layout. Similar frames don't necessarily have to become separate files in the final product.

| Sheet | State/reaction | Suggested timings for frames 1–6, in milliseconds | Behavior |
| --- | --- | --- | --- |
| 01-rest.png | Blink | 1800, 90, 110, 90, 300, 1600 | An occasional blink; between sequences hold the neutral pose, with no continuous loop |
| 02-working.png | Exploring gaze | 450, 120, 400, 250, 120, 400 | Once on entering the state, then back to the neutral pose; any repeats spaced out |
| 03-waiting.png | Call and wait | 150, 120, 250, 160, 250, 1000 | Once, then hold the final pose until a real change |
| 04-connection.png | Puzzlement | 150, 200, 250, 300, 300, 1000 | Once, then hold the final pose |
| 05-turn-finished.png | Completion smile | 100, 150, 300, 200, 150, 200 | Once, then back to the current priority state |
| 06-wink.png | Click acknowledgement | 80, 90, 180, 100, 90, 120 | Once on click, without delaying the panel opening |

## State rules

Priority: Waiting for you → Connection problem → Working → Idle. Completion must not hide an attention request. Closed and Removed are conditions of conversations, not robot expressions. Do not disturb limits alerts. With reduced motion, use only each state's representative pose.

## Quality control and usage

The sheets show the desired expressions and transitions. Generation can introduce halos and differences in silhouette, scale, margins and details between frames. Before integration, the casing must be made uniform at the base, centers aligned, margins guaranteed so the ears aren't cropped, the background cleaned up and the actual transparency checked. The images aren't ready yet for reliable automatic cropping, nor to be declared pixel-perfect.

Check the animation at 48, 64 and 96 pixels on light and dark backgrounds. The eyes must tell working, waiting and problem apart even without motion. Reuse identical frames and keep timings separate from the images. The timings above are proposals to refine in an animated preview.

The prompts.json file keeps the exact prompts used. No GIF animation or app integration is included in this package.

## Update available — v3 addition

`12-update-available.png`: the robot looks up, a small empty bubble appears, then the “new me available!” bubble while the robot smiles, closes its eyes happily, winks and goes back to neutral. Timings 400, 160, 650, 450, 350, 900 ms.

In the app the bubble is a separate layer (`fumetto-piccolo`, `fumetto`), extracted from frames 2 and 6 and overlaid on the robot. This way the face keeps showing the operational state, and the bubble stays until the user clicks the companion. During the new conversation “!” the bubble hides, because they take up the same space. The companion window grows upward only while the bubble is visible.

## Gazes toward the panel — v2 addition

Two additional ImageGen sheets, six poses each. Directions refer to the screen as seen by the user, not to the robot's anatomical right.

| Sheet | Frame 1 | Frame 2 | Frame 3 | Frame 4 | Frame 5 | Frame 6 |
| --- | --- | --- | --- | --- | --- | --- |
| 07-look-down.png | Down center | Down, transition right | Down right | Down center | Down, transition left | Down left |
| 08-look-up.png | Up center | Up, transition right | Up right | Up center | Up, transition left | Up left |

### Use in the flow

- Companion above the panel: looks down. Companion below: looks up.
- If it's aligned with the center of the panel, choose the direction based on the selected card's position relative to the center of the face; also use the horizontal gazes already available. Don't cover cards or controls.
- These are directional poses, not new operational states: they don't replace the meanings of Working, Waiting for you and Connection problem. An attention request takes precedence over simple visual exploration.
- When the panel appears, look at the selected card, or at the center of the content if nothing is selected. Change gaze when the selection or keyboard focus changes, without constantly chasing the pointer.
- Hold the pose until the target changes. For a sideways move use the intermediate pose for about 100–140 ms; a demo scan can use 500, 120, 600, 250, 120, 600 ms. No continuous loop in the product.
- When the panel closes, go back to the current operational state's expression. With reduced motion, switch pose directly, without interpolation or animated movement of the companion.

The sheets are still concepts: halos and small differences between frames are visible. Before use, casing, margins, transparency and scale must be made uniform. The exact prompts are in prompts-gazes.json.
