FLUX.2 prompt format (FLUX.2 Text Encoder). The text encoder is an LLM (Mistral-Small-3.1 for dev, Qwen3 for klein), so write natural ENGLISH sentences — no tag lists, no weights like `(word:1.3)`:

1. Subject first — who or what, concrete visible attributes (clothing, material, color, pose, expression).
2. Setting and composition — where, surroundings, framing (close-up / medium / wide), viewpoint.
3. Light and atmosphere — time of day, light source, weather, mood.
4. Medium and style — photograph (lens, depth of field), illustration, 3D render …

Rules:
- No negative prompt. dev: `guidance` (default 4) is prompt adherence, 3–5 is the useful range. klein (flux.2-klein-4b / -9b): step-distilled, 4 steps, guidance is ignored — keep Sampler `steps` at 0 (= model default: dev 50, klein 4).
- Text to render goes in double quotes, verbatim: `a poster that says "GRAND OPENING"`.
- The prompt can be long and structured (up to 512 tokens); FLUX.2 follows detailed layouts and colors (hex codes work).
- Size: FLUX Empty Latent (sides multiple of 16, ~1 MP sweet spot, up to 4 MP). Without Empty Latent the Sampler takes the size of the first reference.
- Editing: FLUX.2 Reference nodes feed pictures into the Sampler (`references`). Chain them (Reference.references → next Reference) for several images; in the prompt say "image 1", "image 2" in chain order. Write the edit as an instruction: what to change AND what to keep ("replace the sky with a sunset, keep the people and the car unchanged").
- Memory: whatever does not fit in VRAM streams from RAM/disk automatically; dev on a small card is slower (seconds per step), klein is fast.
