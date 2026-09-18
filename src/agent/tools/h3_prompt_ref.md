MiniMax-H3 reference prompt format (Ref2VA). The model sees the references under the tags listed above (`<Picture N>`, `<Video N>`, `<Audio N>`) — use exactly those tags. Look at the references first (view_media) so the descriptions match what is really in them. Write the prompt in ENGLISH, six sections in this exact order:

subject_definitions:
One line per thing that must be tracked. Tags:
- `<Subject N>` — reusable visible content taken from a reference: a person, animal, object, scene, outfit, style, motion. Name its source and main features: `<Subject 1> is the young woman in <Picture 1>, with long dark hair and a blue cardigan.` One subject may combine sources: `… whose appearance comes from <Picture 1> and whose walking motion comes from <Video 1>.`
- `<Picture N>` on its own line only when the image itself is a first frame, last frame, keyframe or storyboard: `<Picture 2> is the first frame of [Shot 1], showing …`. An image used only to define a subject is cited inside that subject's line.
- `<Video N>` — whole-video relations only: `<Video 1> is the source video for the editing task.` / continuation from its end / reference for camera movement, cuts, rhythm.
- `<Audio N>` — an audio clip or a video's own soundtrack: `<Audio 1> is the synchronized audio track of <Video 1>, providing the background music.` / `<Audio 2> is the voice-timbre reference for <Subject 1> (S1).`

summary:
`[task type]` in brackets (e.g. `[subject reference]`, `[video editing + audio reference]`, `[video continuation]`, `[motion reference]`), then 2–3 sentences: what the target video is and the main reference relations.

retention_analysis:
One line per defined item — where it appears and how it is kept: `<Subject 1> (appears in [Shot 1]): fully_preserved - identity, hair, outfit are kept, the mouth is newly animated.` Statuses: fully_preserved / partially_preserved / transferred; for audio: fully_copy / partially_copy / reference.

detailed_description:
`The target video is in <style> style.` then `[Shot 1] …` — as detailed and explicit as possible: composition, subject appearance and position, environment and lighting, actions and state changes, camera movement, sound, and the exact moment each referenced item appears. Later shots: `[Shot 2] At 00:03.500, the camera cuts to …`, times strictly increasing and inside the duration (4–15 s). Camera as a sentence: Push In/Pull Out, Pan, Truck, Tilt, Arc Shot, Tracking Shot, Static Shot + `with small/large amplitude` + `at slow/fast speed`. Speech: stable speaker IDs `(S1)`; inside `<d>` only the language tag and the exact words, never translated: `<Subject 1> (S1) speaks softly, <d>[Russian] Иди за ветром.</d>`.

overall_soundscape:
Ambience and physical sounds; say which `<Audio N>` is reused or referenced.

non_diegetic_music:
Background music only the audience hears (or the reused `<Audio N>`), or `None.`

Keep every tag's meaning identical across all six sections; never mention a tag that is not in the list above.
