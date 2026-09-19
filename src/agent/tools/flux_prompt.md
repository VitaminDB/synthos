FLUX.1 prompt format (FLUX Text Encoder). Write the prompt in ENGLISH, natural sentences, no tag lists and no weights like `(word:1.3)`:

1. Subject first — who or what, with concrete visible attributes (age, clothing, material, color, pose, expression).
2. Setting and composition — where, what surrounds the subject, framing (close-up / medium / wide), viewpoint.
3. Light and atmosphere — time of day, light source and direction, weather, mood.
4. Medium and style — photograph (lens, depth of field), oil painting, watercolor, 3D render, anime …

Rules:
- There is no negative prompt and no CFG: FLUX.1-dev is guidance-distilled. Say what should be there, not what should not. The Sampler's `guidance` (default 3.5) is prompt adherence: 2.5–3 looks more natural, 4–5 follows the text more strictly.
- Text to render in the image goes in double quotes, verbatim: `a neon sign that says "OPEN"`.
- One scene per prompt; the T5 encoder reads up to 512 tokens, so a detailed paragraph is fine.
- Size lives in FLUX Empty Latent (sides multiple of 16, about 1 MP is the sweet spot; up to ~2 MP). For a frame that feeds a video (LTX / H3 keyframe), match the video's aspect ratio — 16:9 → 1344×768.
- image→image: FLUX VAE Encode takes the source picture; Sampler `denoise` 0.3–0.5 keeps the composition, 0.6–0.8 repaints more, 1.0 ignores the source.
- FLUX.1-schnell (if that is the bundle): 4 steps, guidance is ignored.
