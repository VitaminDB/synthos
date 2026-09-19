/* FLUX.1 (Checkpoint, Text Encoder, Empty Latent, VAE Encode, Sampler,
 * VAE Decode) и ноды картинок (Image, Image Save).
 *
 * Палитра — тёплая оранжевая, в тон проводу `image` (#F97316), чтобы граф
 * картинок отличался от янтарно-пурпурного H3 и сине-бирюзового LTX.
 * Field-row'ы — общие acestep-классы. */

.flux-node,
.image-node {
    width: 380px;
}

.image-node-preview {
    width: 360px;
    height: 240px;
    border-radius: 6px;
    background: #120d0a;
}

.flux-node-info {
    font-size: 11px;
    color: #b09a8f;
    padding: 2px 0 0 0;
}

.flux-node-running {
    font-size: 11px;
    color: #f59e4b;
}
