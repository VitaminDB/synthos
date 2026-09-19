SDXL prompt format (SDXL Text Encoder, two CLIP encoders). CLIP reads at most 77 tokens, so keep the prompt SHORT and front-load what matters:

1. Subject with key visible attributes, then setting, then light, then medium/style ("oil painting", "35mm photograph", "digital illustration").
2. Comma-separated phrases are fine (CLIP is not an LLM); quality words like "highly detailed" help a little.
3. Negative prompt (`negative` input, optional): what to avoid — "blurry, low quality, deformed hands, watermark, text". Empty negative = zeros (the pipeline default).

Rules:
- Size: FLUX Empty Latent, sides are rounded down to multiples of 64; SDXL is trained at ~1 MP (1024×1024, 1152×896, 1344×768…) — much larger sizes duplicate objects.
- Sampler: 30 steps and guidance 5 are the defaults (guidance 4–8 useful); SDXL renders text poorly.
- image→image: SDXL VAE Encode takes the source picture; Sampler `denoise` 0.3–0.5 keeps the composition, 0.6–0.8 repaints more.
