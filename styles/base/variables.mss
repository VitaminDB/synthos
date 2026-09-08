/* M3-flavoured design tokens for app/synthos.
 * Keep everything that tunes the look (colours, radii, spacings, shadows)
 * in one place so the app code only carries layout logic. */

:root {
    /* Window shell */
    --bg-window:        #EEECF2;
    --bg-shell:         #FFFFFF;
    --shadow-shell:     0 18px 38px rgba(24, 24, 43, 0.10);

    /* Panels */
    --bg-rail:          #FAFAFC;
    --bg-chats:         #FFFFFF;
    --bg-chat:          #FFFFFF;
    --bg-chat-dots:     #E6E6EE;
    --bg-panel:         #FFFFFF;
    --bg-search:        #F4F4F7;

    /* Frosted-glass tokens (overlay-карточки: voice-панель и пр.).
     * Парсер MSS не раскрывает var() внутри rgba(), поэтому полный rgba —
     * единственный валидный способ хранить полупрозрачные значения. */
    --glass-card-bg:    rgba(255, 255, 255, 0.62);
    --glass-field-bg:   rgba(255, 255, 255, 0.50);
    --glass-border:     rgba(28, 30, 38, 0.10);
    --glass-shadow:     0 24px 64px rgba(20, 24, 40, 0.20);

    /* Accents */
    --primary:          #EE5E48;
    --primary-hover:    #E04A33;
    --primary-soft:     #FFECE6;
    --on-primary:       #FFFFFF;

    /* Text */
    --text:             #1C1D22;
    --text-muted:       #6B7280;
    --text-subtle:      #9CA3AF;
    --text-inverse:     #FFFFFF;

    /* Surfaces / state */
    --surface-hover:    #F3F3F5;
    --surface-selected: #FDF1EE;
    --border:           #E5E7EB;
    --border-soft:      #EEF0F3;
    --border-strong:    #D6D8DE;

    /* Semantic state */
    --error:            #E55353;
    --error-hover:      #C03E3E;
    /* «Готово»/«сделано»: галочка закрытой карточки, флаг колонки done. */
    --success:          #2F9E63;
    --warning:          #E8A33D;
    --warning-soft:     #FCF1DF;

    /* Diff (code editor conflict / history) */
    --diff-added:       #1A7F37;
    --diff-added-bg:    #E6F4EA;
    --diff-removed:     #C0362C;
    --diff-removed-bg:  #FBE9E7;

    /* Presence / tags */
    --presence-online:       #22C55E;
    --presence-offline:      #9CA3AF;
    --presence-instagram:    #E1306C;
    --presence-messenger:    #3B5BFF;
    --presence-whatsapp:     #25D366;
    --presence-telegram:     #229ED9;

    /* Avatar tones */
    --avatar-orange:  #F3B086;
    --avatar-green:   #B7DEC2;
    --avatar-blue:    #B7CCE8;
    --avatar-violet:  #C9B8E3;
    --avatar-rose:    #F3B9BE;
    --avatar-slate:   #D3D6DD;

    /* Radii & spacing */
    --radius-shell:    20px;
    --radius-panel:    16px;
    --radius-pill:     999px;
    --radius-bubble:   18px;

    --spacing-xs:      4px;
    --spacing-sm:      8px;
    --spacing-md:      12px;
    --spacing-lg:      16px;
    --spacing-xl:      24px;

    /* Motion */
    --ease-standard:   cubic-bezier(0.2, 0.0, 0.0, 1.0);
    --duration-fast:   120ms;
    --duration-med:    200ms;
}
