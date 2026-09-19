Qwen-Image edit prompt format (Qwen-Image Text Encoder). The encoder is Qwen2.5-VL: it SEES the pictures from the Reference chain together with your text, so write an INSTRUCTION, not a scene description:

1. What to change — concrete and visible ("replace the red car with a blue bicycle", "make it night with street lights on", "turn the photo into a watercolor painting").
2. What to keep — name it explicitly ("keep the person's face, pose and clothing unchanged", "keep the composition").
3. Text edits: quote the exact text — `change the sign text to "OPEN 24/7"`; Chinese and English text render well.

Rules:
- English or Chinese both work; one edit per prompt works best — chain runs for several edits.
- Several pictures (Qwen-Image-Edit-2509/2511 only, up to 4): chain Qwen-Image Reference nodes (references → next Reference); in the prompt they are "Picture 1", "Picture 2"… in chain order ("the man from Picture 1 sits on the sofa from Picture 2"). The original Qwen-Image-Edit takes exactly one picture.
- The same Reference chain goes into BOTH the Text Encoder (`references`) and the Sampler (`references`).
- Output size: ~1 MP in the aspect ratio of the (last) picture; FLUX Empty Latent on the Sampler `latent` input overrides it.
- Sampler: `cfg` 4 (true CFG with the negative prompt, default " "), steps 0 = model default (Edit 50, 2509/2511 40); cfg 1 disables CFG and is twice as fast but follows the instruction less.
- Memory: DiT is 20B parameters; whatever does not fit in VRAM streams from RAM/disk automatically (slow on a small card).
