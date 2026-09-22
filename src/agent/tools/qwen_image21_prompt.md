Qwen-Image 2.1 prompt format (Qwen-Image 2.1 Text Encoder). One model does text→image, editing by references and transparent RGBA. The encoder is Qwen3-VL: it SEES the pictures from the Reference chain together with your text.

Text→image (no references): describe the finished picture as an observer — subject, setting, lighting (quality, direction, colour), materials and textures, camera/framing, palette; name colours with a modifier, enumerate objects, keep physics consistent. Text to render goes in double quotes exactly as it must appear ("OPEN 24/7"); English and Chinese text render well. Write in English; only quoted in-image text keeps its own script.

Editing (references connected): write an INSTRUCTION — what to change (concrete and visible) and what to keep ("keep the person's face, pose and clothing unchanged"). Refer to references as <image1>, <image2>… in chain order ("place the mug from <image1> on the desk from <image2>"); with a single reference just say "the image".

Transparent output: start the prompt with "This is an RGBA image with transparency." and end with "The image has alpha channel and the background is transparent." — the VAE Decode yields RGBA and Image Save writes a PNG with alpha. Transparent input PNGs are edited with their alpha.

Rules:
- The same Reference chain goes into BOTH the Text Encoder (`references`) and the Sampler (`references`); up to 10 references.
- Size: FLUX Empty Latent on the Sampler `latent` input (sides multiple of 32; native 1024² and 2048²), otherwise the Checkpoint `resolution` (1024 by default) — a square for text→image or the aspect ratio of the last reference for editing. References are resized to that resolution too.
- Sampler: steps 0 = 40 (model default); `cfg` 1 = no CFG (the model is meant to run without it); cfg > 1 with a negative prompt enables true CFG and doubles the time; `kv_cache` on — text and references are computed once, later steps only denoise the target.
- Memory: DiT is 7B parameters (mxfp8 ~7.4 GB, nvfp4 ~3.9 GB, bf16 14 GB); whatever does not fit in VRAM streams from RAM/disk automatically.
