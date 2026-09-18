MiniMax-H3 prompt format (text→video, first/last frame). Write the prompt in ENGLISH, as one text in this exact order:

1. Keyframe instruction — first line, then a blank line. Omit it for plain text→video.
   - first frame only: `For the target video, at 0.00 seconds into the target video, <Picture 1> (from [Shot 1]) is fully referenced.`
   - first + last frame: `How the reference pictures align with the target video — Picture 1 (from Shot 1) aligns with the 0.00-second mark of the target video; Picture 2 (from Shot N) aligns with the S.SS-second mark of the target video.`
   - last frame only: `How the reference pictures align with the target video — <Picture 1> (from [Shot N]) aligns with the S.SS-second mark of the target video.`
   N = index of the final shot, S.SS = video duration with two decimals (e.g. 5.17).
2. `integrated_multimodal_description: [Shot 1] …` — the timeline: visual style first (Live-action, cinematic / 2D-animated / 3D CG / claymation …), composition, subject appearance and position, environment and lighting, actions and reactions, camera, synchronized sound. Concrete visible and audible details, not a plot summary. With a first frame: start from what is in the picture and develop forward; with first+last: describe the continuous path between them, normally one shot.
3. `overall_soundscape: …` — ambience, physical sounds, non-verbal human sounds over the whole video.
4. `non_diegetic_music: …` — background music only the audience hears, or `None.`

Rules:
- Shots: `[Shot 1]` has no timestamp; later ones start with the cut time, strictly increasing and inside the duration: `[Shot 2] At 00:03.500, the camera cuts to …`. Total timing must fit the requested duration (4–15 s).
- Camera as a natural sentence: motion type (Zoom In/Out, Push In/Pull Out, Pan Left/Right, Truck Left/Right, Tilt Up/Down, Pedestal Up/Down, Arc Shot, Tracking Shot, Static Shot, POV, Roll) + optional `with small/large amplitude` + `at slow/fast speed`.
- Speech: every speaking or singing subject gets a stable ID `(S1)`, `(S2)`; describe the voice outside `<d>`; inside `<d>` only the language tag and the exact words, never translated: `The young woman with a quiet, breathy voice (S1) says: <d>[Russian] Я выхожу на следующей.</d>`. Voiceover: `says in an off-screen voiceover: <d>…</d> while his lips remain completely closed.`
- Text visible on screen goes in English double quotes, verbatim.
