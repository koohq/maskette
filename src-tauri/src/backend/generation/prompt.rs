//! The image prompts, for the hosted image APIs (OpenRouter, Ark,
//! SiliconFlow).
//!
//! The wording is the old backend's, unchanged: the stage prompts of
//! `app/generation/backends/prompting.py` (with the realism proposals 1-3,
//! drawn from photos of finished masks), the per-mode constraint lists of
//! `app/generation/job_store.py`, and the assembly and shortening of
//! `app/generation/backends/image_api_prompt.py`.
//!
//! Neither repository ships a finished-product style photo
//! (`ref/product-reference.png`), so only the "no style photo" variants of
//! the prompts moved. Single lines and the constraint lists can be overridden
//! in `prompts.json` ([`crate::backend::prompts`], the old
//! `runtime/prompts.json`).

use serde_json::Value;

use super::{GenerationRequest, Mode};
use crate::backend::prompts::Overrides;
use crate::backend::safety::sanitize_user_text;

// Shared wording adapted from the upstream KigCraft V2 stage briefs
// (app/conversation/stages.py, GPL-3.0-or-later).

/// Pose rule for every front image (2D design and head shell). Four-view
/// sheets state their own angles.
pub const HEAD_POSE: &str = "Head pose: the head is upright and faces the camera straight on, with no tilt, roll, turn or nod: the face's \
midline is vertical, the eyes and the mouth line are level, the chin is level, and the head is centred and \
symmetric, with both eyes at exactly the same height mirrored across the face centerline and the nose and mouth \
on that centerline. This holds even when a reference shows the character at an angle, tilted, turned, looking up \
or down or leaning: rebuild the head upright and frontal, inferring the hidden side conservatively from the visible \
hair and features, and never copy a three-quarter, turned or tilted pose from the references.";

/// Uneven eyes in a reference (one higher, lower, larger or differently
/// shaped) are nearly always an accident of the art or its angle, and the
/// "keep the eyes as drawn" lines made the models copy them. Every stage
/// rebuilds them as one mirrored pair, but keeps what is meant to differ:
/// two iris colours, or a wink. From VSK (`EYE_SYMMETRY_LINE`,
/// GPL-3.0-or-later), with that exception added. Since 2026-11-17 it only
/// moves and resizes the eyes: "rebuild the pair" let the model swap the
/// drawn eyes for stock doll eyes.
pub const EYE_SYMMETRY_LINE: &str = "Eye placement: the two eyes are one level, mirrored pair of the same size (in the turned views of a four-view \
sheet, at the same height and size as in the front view). If the design draws them at slightly different heights or \
sizes, only move or resize them into that pair; keep the drawn eye artwork, each iris colour (a character with two \
different eye colours keeps both) and any wink or other one-sided expression the design clearly shows unchanged.";

/// How a head-shell image should look: a photograph of a physical object,
/// not an illustration. Carries the realism proposals 2 (satin sheen) and 3
/// (pale skin) and the open-mouth half of proposal 1. The workshop
/// translation at the end (2026-11-17) makes the workshop look and the
/// faithfulness to the drawing point the same way: the maker copies the
/// drawing's eyes, hairstyle and colours, instead of building a stock doll
/// face; doll and figure words (porcelain, resin, sculpted sockets) were
/// taken out for the same reason.
pub const HEAD_SHELL_LOOK: &str = "Look: a photorealistic studio product photograph of a real, physical, hand-made kigurumi head shell wearing its wig, \
the kind of catalogue photo a professional maker shoots for a commission. Shot on a full-frame camera with an 85 mm \
lens at f/8, tack sharp across the face and fringe with only a gentle falloff toward the far hair ends, true colour, \
natural dynamic range with soft highlights and open shadows, fine real-world surface texture. \
It must not read as anime art, an illustration, a 3D render, a figure, a vinyl doll or a digital painting: no outlines or ink \
lines, no cel shading, no flat colour fills, no soft digital-painting shading on the skin or hair, no painted shine \
streaks on the hair, no glow or bloom. \
Anime proportions and the character's design stay, but everything is built from real materials with real depth. \
Studio set-up: a seamless light grey-white paper backdrop, bright soft light from the front and slightly above - \
a large soft-box key with gentle fill from the sides - so the face is evenly lit and easy to read, with only soft \
form shadows under the fringe, the wig and the chin, and at most a faint rim light separating the wig from the \
backdrop. Avoid dramatic side lighting: neither side of the face may fall into heavy shadow. \
Materials: the shell is smooth fibreglass (FRP) with an even, sprayed satin coat - even \
and smooth, with a broad, soft sheen where the key light grazes the forehead, the nose tip and the cheeks, never \
wet, mirror-glossy or specular - in a pale skin paint with a light \
pink or peach undertone (unless the design gives the skin another colour), a soft airbrushed blush across the \
cheeks following the design (a smooth \
gradient only, never hatch marks, stroke lines or drawn-on anime blush), and only \
the shallow relief of an animegao mask (a smooth rounded face, a tiny nose tip, no \
modelled lips or cheekbones), so the face reads as a solid object, not a flat \
drawing. The eyes are separate eye parts set into the shell: the design's eye artwork - iris colours and shading, \
pupil shape, drawn sparkles, upper lash line and lashes - printed exactly as drawn, under a clear glossy dome that \
adds one or two soft rectangular window reflections on top of the printed sparkles, with a clean, shallow rim where \
the shell meets each eye part. Brows, lashes, blush and a closed mouth are matte paint sitting on the surface, and a \
closed mouth in particular is one flat painted stroke flush with the shell: not carved, engraved or sculpted into \
it, with no groove, incised seam or dark inner shadow, at most the soft edge of the paint itself, small in the \
face as drawn - a thin darker stroke with tiny corner ticks. Only when the design shows an open mouth is it a \
real opening: a small, shallow recess with a smooth rounded rim, painted red or pink inside, holding the tongue, \
teeth or fangs the design shows as small sculpted parts, and no larger than drawn. The wig is \
heat-resistant synthetic fibre with a satin sheen, never wet-look or waxy gloss: the wig reads as styled fibre \
cloth, with strands grouped into clumps that keep a full, rounded silhouette, each clump showing fine matte \
fibres along its length, and soft broad highlight bands curving along the clumps instead of bright specular \
streaks on single strands; spiky styles are built from grouped clumps, never from sparse ribbon-like strands \
flying apart, the roots are darker and the layers \
underneath fall into shadow, there is natural volume and a believable parting where the \
wig meets the shell; braids show the woven strand structure with a highlight on each ridge; the fringe casts a soft \
shadow on the forehead. The maker sets the wig by matching the part, the height of the roots above it, and the \
large locks that define the outline in front view: where the drawing lifts the fringe roots from the part before \
the locks fall, that lift is set firmly with heat and spray, so the fringe keeps the drawn sculpted outline instead \
of dropping flat from the crown. Follow each defining lock from its root through its bend to the place where its \
tip meets the brow, the lash line, the cheek or the jaw in the reference. Preserve blunt sections as a continuous \
edge and pointed sections as distinct tips. Fine fibres run together along each styled lock and share a soft \
highlight, and flyaway strands stay inside the hairstyle silhouette. Match which portions of the eyes and cheeks \
the drawn fringe and side locks actually expose; only the design itself may put hair over an eye. \
Colours stay true to the character, rendered with real-world tonal range, soft contact \
shading between hair and face, and fine surface detail everywhere. \
Workshop translation: this is the approval photo a kigurumi workshop sends its client before delivery, so it \
matches the design sheet part by part. The maker copies the drawing, not a stock doll face. The eye parts carry \
the design's own eye artwork, printed exactly as drawn - the same iris colours and shading (for example a darker \
upper iris), the same pupil shape, the same sparkle shapes and lash shapes - under a clear glossy lens that adds \
one or two soft window reflections. The skin is one smooth, evenly sprayed coat in the design's skin colour; \
blush, nose and mouth are only as strong as drawn. The wig is cut and heat-set by the maker to the drawn \
hairstyle: tidy, deliberate locks with the drawn lengths and tip shapes, fine fibres visible up close, no frizz. \
Its fibre colour matches the design's hair colour in the lit areas - the same lightness and hue, not darker, \
browner or more saturated.";

// Compact versions of the longest stage paragraphs, for image models with a
// short prompt limit (Grok Imagine takes 8,000 characters). Used only when
// the full prompt does not fit; see `build_image_prompt`.
pub const HEAD_SHELL_LOOK_COMPACT: &str = "Look: a photorealistic studio product photograph of a real, hand-made kigurumi head shell wearing its wig, the \
catalogue photo a professional maker shoots for a commission: full-frame camera, 85 mm lens at f/8, tack sharp, \
true colour, soft highlights and open shadows. Never anime art, an illustration, a 3D render, a figure or a doll: \
no outlines, cel shading, flat fills, painted shine streaks on the hair or glow. Light grey-white seamless paper \
backdrop, a large soft-box key from the front and slightly above with soft side fill, so the face is evenly lit \
with only soft shadows under the fringe and the chin. Shell: smooth fibreglass with an even, sprayed satin coat \
(never wet or mirror-glossy), pale skin paint, a soft airbrushed blush only as strong as drawn, only shallow \
relief. Eyes: separate eye parts carrying the design's own eye artwork (iris colours and shading, pupil shape, \
sparkles, lashes) printed as drawn under a clear glossy dome with one or two soft window reflections; never stock \
doll eyes. Brows, blush and a closed mouth are flat matte paint, never carved; an open mouth is a small \
shallow opening, red inside. Wig: heat-resistant synthetic fibre with a satin sheen, cut and set to the drawn \
hairstyle in tidy clumps of fine fibres with soft highlight bands; the fringe keeps the drawn root lift, length \
and tip shapes, never as locks over the iris. Colours, the hair's lightness included, stay true to the character.";

pub const HEAD_POSE_COMPACT: &str = "Head pose: upright and facing the camera straight on, with no tilt, turn or nod; the eyes and the mouth line are \
level and mirrored across a vertical face centerline. When a reference shows the head at an angle, rebuild it \
frontal and never copy that angle.";

pub const HEAD_SHELL_PRESENTATION_COMPACT: &str = "Presentation: the head shell floats in mid-air with no mount, mannequin, neck, base or surface under it and no \
floor shadow; it ends at its natural jaw edge. Nothing below the head appears: no shoulders, body or clothing, \
even when a reference shows a collar, shirt, jacket or neck ribbon. A hood is clothing too: leave it out and show \
the wig's crown and hairline that lay under it; hats, headbands and hairpins stay as drawn. Loose hair, braids and \
tails hang straight \
down under gravity into \
open air. The fringe and side locks end where the drawing places them and frame the face without covering it; \
long hair falls beside or behind the shell. Only the design itself may put hair over an eye.";

/// Shared by every head-shell image (front, revisions and four-view) so they
/// all hang the same way.
pub const HEAD_SHELL_PRESENTATION: &str = "Presentation: the head shell is photographed floating in mid-air, as if suspended by an invisible support. \
No mount, mannequin, bust, table, floor, shelf or any other surface touches or supports it, and there is no \
floor or contact shadow beneath it (at most a faint soft shadow far behind it on the backdrop). The shell is a \
hollow head: below the face it simply ends at its natural jaw and opening edge, with no neck stub and no base. \
Nothing below the head appears: no shoulders, body or clothing. Even when a reference shows a collar, shirt, \
jacket, neck ribbon or tie, leave it out entirely; the photograph shows the head shell and its wig alone. \
A hood attached to the clothing is part of the clothing, not a head accessory: leave it out as well, and show the \
crown of the wig, its hairline and the roots of its fringe in a shape that reads naturally as the hair that lay \
under the hood, continuing the fringe and side locks the drawing shows. Accessories worn on the head itself, such \
as hats, headbands and hairpins, stay exactly as drawn. \
The wig hangs freely under gravity: loose hair, braids, twin tails and ponytails fall straight down with \
natural weight, pass beyond the bottom of the shell through open air, and never rest on, bend against or pile up on \
any surface; long braids end in the air with their tips pointing down. The ends of the fringe and the side locks \
rest where the drawing places them, with the same overlap with the cheeks and the same visibility of each eye as \
drawn, and long hair falls beside or behind the shell, never in front of the face.";

pub const WATERMARK_LINE: &str = "Reference and base images may carry a watermark: faint tiled text and an AI-generated notice in a corner. Ignore it \
as if it were not there and never reproduce it. Do not generate any watermark, text, logo, signature, UI, labels, \
captions or extra characters; the service will add the configured watermark after generation.";

pub const IDENTITY_LINE: &str = "Preserve the character identity, eye color, eye shape, expression, facial mood, and clearly visible head \
accessories or special features. Do not invent features that are not in the references and do not average the \
character into a generic face.";
pub const HAIR_FIDELITY_LINE: &str = "Faithfully reproduce all visible hairstyle details: hair silhouette, bangs/fringe shape, side locks, ahoge, strand \
grouping, layered clumps, parting, volume, length, asymmetry, hair accessories, and color blocks or highlights. \
Do not simplify, invent, or replace visible hairstyle details. The hair outline is no wider or fuller than drawn.";
pub const NO_IMPOSED_HAIRSTYLE_LINE: &str = "Do not impose a specific hairstyle such as twin tails, long hair, short hair, bangs, or hair-length restoration \
unless it is clearly visible in the references or explicitly requested by the user.";
/// Ears as drawn, not as prominent as possible: the old "must be visible,
/// never hidden" wording made the models enlarge them and push them up
/// above the head (2026-11-17).
pub const FRONT_EARS_LINE: &str = "Keep the character's characteristic ears (elf/pointed ears, animal ears, or horn-like head appendages) exactly as \
designed: the same shape, the same size relative to the head and the same position, including how high they sit. \
Where the design draws the ear tips level with the top of the hair, they stay level with it and do not stick up \
above the head; ears drawn small or partly in the hair stay that small. Never enlarge, raise or spread the ears to \
make them more prominent. In the front view both ears are present and symmetric; if the reference only shows one \
side, mirror it to complete the other side. Never remove, merge or crop the ears.";
pub const SHELL_EARS_LINE: &str = "Keep the character's characteristic ears (elf/pointed ears, animal ears, or horn-like head appendages) exactly as \
designed: the same shape, the same size relative to the head and the same position, including how high they sit. \
Where the design draws the ear tips level with the top of the hair, they stay level with it and do not stick up \
above the head; ears drawn small or partly in the hair stay that small. Never enlarge, raise or spread the ears to \
make them more prominent. In the front view both ears are present and symmetric; if the reference only shows one \
side, mirror it to complete the other side. Never remove, merge or crop the ears, even when a finished-product style \
reference shows a shell without ears.";

/// What the drawn design becomes on the physical product. Without it the
/// model kept the illustration's rendering and produced a shaded drawing
/// instead of a photographed object.
pub const PHYSICAL_TRANSLATION_LINE: &str = "The design is drawn art; the output is a photograph of the real object a maker builds from it. Keep what the \
design shows (shapes, proportions, colours, eye design, hairstyle, accessories) and replace how it is drawn: the \
face becomes a smooth painted shell with the design's anime face proportions, the drawn eyes become \
glossy domed lens eyes whose iris print copies the drawn iris, the drawn hair becomes a synthetic-fibre wig with \
real strands, volume and shadowed under-layers, and accessories become real resin, metal or fabric parts. Never \
keep line art, cel shading, painted highlights or the illustration's soft airbrushed skin.";

/// Realism must stop at the materials: a kigurumi face is the drawing's anime
/// face made solid. Without this the head-shell photos drifted toward a real
/// person (sculpted nose, lips, smaller eyes).
pub const ANIME_FACE_LINE: &str = "Face: an animegao kigurumi face is the design's anime face made solid, not a human face. Keep the design's face \
shape and proportions and the eyes' size, shape and placement exactly as drawn (large anime eyes, not shrunk to \
human size). The nose is only a tiny rounded tip or a small painted shadow and the mouth a short painted line or \
small shape as drawn; a closed mouth is painted flat on the surface: never a carved, engraved or incised groove, \
with no depth, inner shadow or cast shadow around it (an open one: a small shallow opening). No lips, \
philtrum, nostrils, cheekbones, eyelid folds or other realistic \
human facial anatomy. It must read as a painted mask of the drawn character, never as a real person, a cosplayer \
or a realistic 3D render. Keep the drawn vertical spacing of the face: the nose, mouth and chin stay as close under \
the eyes as drawn, so the lower face stays as short and young as in the design; the eyes keep the drawn opening \
height and lid shape, not rounder or wider open.";

/// The front style line when no finished-product photo is attached.
pub const FRONT_NONE_STYLE_LINE: &str = "No finished-product reference image is attached. Achieve the physical product look from these written \
requirements only: white studio background, finished kigurumi head shell material, wig fiber realism, \
clean product framing, and product-photo lighting.";
pub const TURNAROUND_NONE_STYLE_LINE: &str = "No finished-product reference image is attached. Build the four-view layout and the physical product look \
from the written requirements only: four evenly spaced views, white studio background, finished shell \
surface, and wig fiber realism.";

pub const FOUR_VIEW_LAYOUT_LINE: &str = "Layout: one horizontal sheet showing the SAME head from four directions, side by side in a single row, left to \
right: (1) front, facing the camera straight on; (2) front three-quarter, turned about 45 degrees to the \
character's left, so the face is still clearly visible; (3) side profile, turned 90 degrees; (4) back, turned 180 \
degrees, the back of the head and hair with no face visible. Within each view the head is upright, with no tilt, \
roll or nod; only the turn differs. The four heads are the same size and scale, evenly spaced, aligned on one \
common horizontal baseline, centred vertically, with clear margins around each and no overlap or cropping. Left and \
right belong to the head, not to the picture: a feature on the character's left side (the viewer's right in the front \
view) stays on the character's left in every view. In the back view the sides are mirrored, so it appears on the \
viewer's LEFT there; in the side profile it is on the far side of the head, hidden or only partly visible behind it. \
A one-sided feature, such as a single braid, side ponytail, side lock or hairpin, keeps the same distance from the \
centre line of the head in every view and is never moved to the middle of the back.";
pub const FOUR_VIEW_CLEAN_LINE: &str = "Draw no text, labels, arrows, grid lines, panel borders or extra heads, and do not output four separate images.";

const COMPOSITION_LINE: &str = "Composition: the whole head shell, the full wig silhouette and both ears are inside the frame, centred, with clear \
margins; head and wig only, no body.";

/// The shared lines of the stage prompts, each one overridable by its key
/// in `prompts.json` (see [`crate::backend::prompts`]).
pub struct Texts<'a> {
    head_pose: &'a str,
    head_pose_compact: &'a str,
    eye_symmetry: &'a str,
    watermark: &'a str,
    identity: &'a str,
    hair_fidelity: &'a str,
    no_imposed_hairstyle: &'a str,
    front_ears: &'a str,
    shell_ears: &'a str,
    physical_translation: &'a str,
    anime_face: &'a str,
    head_shell_look: &'a str,
    head_shell_look_compact: &'a str,
    head_shell_presentation: &'a str,
    head_shell_presentation_compact: &'a str,
    front_none_style: &'a str,
    turnaround_none_style: &'a str,
    four_view_layout: &'a str,
    four_view_clean: &'a str,
}

impl<'a> Texts<'a> {
    pub fn new(overrides: &'a Overrides) -> Self {
        let text = |key, default| overrides.text(key, default);
        Self {
            head_pose: text("head_pose", HEAD_POSE),
            head_pose_compact: text("head_pose_compact", HEAD_POSE_COMPACT),
            eye_symmetry: text("eye_symmetry_line", EYE_SYMMETRY_LINE),
            watermark: text("watermark_line", WATERMARK_LINE),
            identity: text("identity_line", IDENTITY_LINE),
            hair_fidelity: text("hair_fidelity_line", HAIR_FIDELITY_LINE),
            no_imposed_hairstyle: text("no_imposed_hairstyle_line", NO_IMPOSED_HAIRSTYLE_LINE),
            front_ears: text("front_ears_line", FRONT_EARS_LINE),
            shell_ears: text("shell_ears_line", SHELL_EARS_LINE),
            physical_translation: text("physical_translation_line", PHYSICAL_TRANSLATION_LINE),
            anime_face: text("anime_face_line", ANIME_FACE_LINE),
            head_shell_look: text("head_shell_look", HEAD_SHELL_LOOK),
            head_shell_look_compact: text("head_shell_look_compact", HEAD_SHELL_LOOK_COMPACT),
            head_shell_presentation: text("head_shell_presentation", HEAD_SHELL_PRESENTATION),
            head_shell_presentation_compact: text(
                "head_shell_presentation_compact",
                HEAD_SHELL_PRESENTATION_COMPACT,
            ),
            front_none_style: text("style_line_replacement.front.none", FRONT_NONE_STYLE_LINE),
            turnaround_none_style: text(
                "style_line_replacement.turnaround.none",
                TURNAROUND_NONE_STYLE_LINE,
            ),
            four_view_layout: text("four_view_layout_line", FOUR_VIEW_LAYOUT_LINE),
            four_view_clean: text("four_view_clean_line", FOUR_VIEW_CLEAN_LINE),
        }
    }

    /// A long stage paragraph's compact version, for image models with a
    /// short prompt limit.
    fn compact_line(&self, line: String) -> String {
        if line == self.head_shell_look {
            self.head_shell_look_compact.to_owned()
        } else if line == self.head_shell_presentation {
            self.head_shell_presentation_compact.to_owned()
        } else if line == self.head_pose {
            self.head_pose_compact.to_owned()
        } else {
            line
        }
    }
}

/// The head-shell front (design and revision), as the image APIs get it:
/// the old `_final_front_view_prompt` without its landmark lines and the
/// agent-only size line, with the "no style photo" line.
fn final_front_view_prompt(t: &Texts) -> Vec<String> {
    let mut lines = vec![
        "You are generating one final front-view studio photograph of a finished, physical animegao kigurumi head shell \
with its wig, translated from the character design."
            .to_owned(),
        String::new(),
    ];
    lines.push(format!(
        "Use the uploaded character image(s) as the primary identity reference. {}",
        t.identity
    ));
    lines.extend(
        [
            "If the design source is a clean flat 2D design, treat it as the authoritative design to translate into the \
physical product, not as an image to redraw: keep its proportions, eyes, expression, hair and accessories.",
            t.physical_translation,
            t.anime_face,
            t.hair_fidelity,
            t.no_imposed_hairstyle,
            t.head_pose,
            t.eye_symmetry,
            t.shell_ears,
            t.front_none_style,
            "",
            t.head_shell_look,
            t.head_shell_presentation,
            COMPOSITION_LINE,
            "",
            "Kigurumi head requirements:",
            "- smooth hand-painted shell with a soft satin paint finish (never wet or mirror-glossy) and pale skin \
unless the design says otherwise, a fixed expression and only the shallow relief of an animegao mask, no \
realistic human skin texture; only the lens eyes are fully glossy",
            "- tiny nose and a simple mouth as drawn; a closed mouth is painted flush on the surface, never carved into \
the shell, and an open mouth is a small shallow opening with a painted interior; no lips, nostrils or human \
facial anatomy",
            "- large anime eyes: the design's own eye artwork printed as drawn under a clear glossy dome; never realistic \
human eyes or stock doll eyes",
            "- wig mounted on the head shell, with realistic fiber texture and a hairstyle derived from the references or user notes",
            "- the face stays fully visible: the fringe lies flat and keeps the length the design shows, the \
side locks frame the face at the sides, and loose strands never drape over the eyes, cheeks or mouth; only the design may cover an eye",
            "- long loose hair must remain continuous and natural; do not create holes, missing chunks, or cutouts in the hair silhouette",
            "- the characteristic ears or horn-like appendages stay present, matched, and symmetric, at the drawn size and height",
            "",
            t.watermark,
            "Output only one front-view head shell photograph.",
            "",
        ]
        .map(str::to_owned),
    );
    lines
}

fn final_turnaround_prompt(t: &Texts) -> Vec<String> {
    let clean_item = format!("- {}", t.four_view_clean);
    [
        "You are generating one final four-view studio photograph sheet of a finished, physical animegao kigurumi head shell.",
        "",
        "Use the uploaded edited front-view design as the locked design reference. The four-view result must strictly \
preserve the approved front-view design: same character identity, same face, same eyes, same expression, same \
visible head accessories, same materials and same overall proportions in all four heads. Do not redesign, \
simplify, beautify, reinterpret, or change the character.",
        t.physical_translation,
        t.anime_face,
        "Faithfully carry over all visible hairstyle details from the approved front-view design into every generated \
view: hair silhouette, bangs/fringe shape, side locks, ahoge, strand grouping, layered clumps, parting, volume, \
length, asymmetry, hair accessories, and color blocks or highlights. Where the sides or the back are not shown \
anywhere, infer them conservatively from the front and the visible hair; do not invent new accessories or a \
different hairstyle.",
        t.no_imposed_hairstyle,
        t.eye_symmetry,
        t.turnaround_none_style,
        "",
        t.four_view_layout,
        "One shared plain white seamless backdrop and the same lighting direction across all four heads.",
        t.head_shell_look,
        t.head_shell_presentation,
        "",
        "Kigurumi turnaround requirements:",
        "- long loose hair must stay continuous across all views without holes, missing chunks, or cutouts",
        "- the characteristic ears or horn-like appendages stay present, matched, and consistent in every view",
        "- consistent approved design across every view",
        clean_item.as_str(),
        "",
        t.watermark,
        "Output only one four-view turnaround image.",
    ]
    .map(str::to_owned)
    .into()
}

fn character_sheet_common<'a>(t: &Texts<'a>) -> [&'a str; 8] {
    [
        "This is stage 1 of the kigurumi workflow: a clean 2D character design sheet of the head. It is NOT the physical \
head shell yet; the user will edit and approve this design, and the head shell will be generated from it later.",
        "The user's reference images are the source of truth. Reproduce the character faithfully: head and face \
proportions, eye shape, iris colour and highlights, eyebrows, expression and temperament, hair colour and length, \
every distinctive hair structure, ear shape, size, placement and colour, and the drawing style of the source. \
Do not invent features that are not in the references and do not average the character into a generic face.",
        t.hair_fidelity,
        t.no_imposed_hairstyle,
        "Remove everything that would interfere with building a head shell: background, body, clothing, props, hands, \
text, effects, and anything covering the head such as hoods, hats, veils or masks. Rebuild the hair and head shape \
those items hide so that it is consistent with the visible hair; infer hidden hair structure conservatively. Keep \
only the small accessories that belong to the character identity (hairpins, ribbons, earrings, small ornaments), \
placed where they sit on the head.",
        "Medium: keep the source rendering. For illustrated sources draw a flat 2D illustration with clean lines. Do not \
turn it into a realistic, 3D, doll, figure or cosplay image, do not moe-ify the face, and do not add 3D depth or \
realistic shading.",
        "Background: plain pure white with even lighting, no shadow, no frame, no labels.",
        "Do not render a physical product: no kigurumi shell, foam, seams, wig fibres, studio photograph or realistic human skin.",
    ]
}

fn character_front_view_prompt(t: &Texts) -> Vec<String> {
    let mut lines = vec![
        "You are drawing one front-view 2D character design of the character's head from the uploaded reference image(s).".to_owned(),
        String::new(),
        format!("Use the uploaded character image(s) as the identity reference. {}", t.identity),
    ];
    lines.extend(character_sheet_common(t).map(str::to_owned));
    lines.extend(
        [
            "",
            "Composition: front-facing, centred, the whole head including the full hair silhouette and ears visible; head and \
hair only (at most a short neck stub), no shoulders or body.",
            t.head_pose,
            t.eye_symmetry,
            t.front_ears,
            "",
            t.watermark,
            "Output only one front-view design image.",
        ]
        .map(str::to_owned),
    );
    lines
}

fn character_revision_prompt(t: &Texts) -> Vec<String> {
    let mut lines = vec![
        "You are revising one front-view 2D character design of the character's head.".to_owned(),
        String::new(),
        "The primary attached image is the current design draft and is authoritative, including any manual edits. Keep it \
as close as possible: same character, face, eyes, expression, hairstyle, colors, framing, and drawing style. Apply \
only the changes the user asks for or that the annotations drawn on the draft point out, then remove those \
annotation marks."
            .to_owned(),
    ];
    lines.extend(character_sheet_common(t).map(str::to_owned));
    lines.extend(
        [
            t.eye_symmetry,
            "",
            "",
            t.watermark,
            "Output only one front-view design image.",
        ]
        .map(str::to_owned),
    );
    lines
}

fn character_turnaround_prompt(t: &Texts) -> Vec<String> {
    let mut lines = vec![
        "You are drawing one 2D character design turnaround sheet of the character's head: front, three-quarter/front-side, \
side, and back views in one image."
            .to_owned(),
        String::new(),
        "Use the primary attached image as the identity and design source. If it is already a four-view sheet, keep its layout \
and design and apply only the requested changes. Use any other attached images only to clarify details the primary \
image does not show, such as the back of the hair."
            .to_owned(),
    ];
    lines.extend(character_sheet_common(t).map(str::to_owned));
    lines.extend(
        [
            t.eye_symmetry,
            "",
            t.four_view_layout,
            "Every view must show the same design; the back view must show a plausible continuation of the visible hairstyle.",
            "If the character has characteristic ears (elf/pointed ears, animal ears, or horn-like head appendages), keep them \
visible, matched, and consistent in every view; never let hair or a view angle remove or hide them.",
            t.four_view_clean,
            "",
            t.watermark,
            "Output only one four-view design image.",
        ]
        .map(str::to_owned),
    );
    lines
}

/// The stage prompt of a mode, one entry per line.
pub fn stage_lines(mode: Mode, t: &Texts) -> Vec<String> {
    match mode {
        Mode::CharacterFront => character_front_view_prompt(t),
        Mode::CharacterRevision => character_revision_prompt(t),
        Mode::CharacterTurnaround => character_turnaround_prompt(t),
        Mode::Turnaround => final_turnaround_prompt(t),
        Mode::FrontDesign | Mode::FrontRevision | Mode::FrontLocalRevision => {
            final_front_view_prompt(t)
        }
        Mode::EyeDetail => EYE_DETAIL_LINES.map(str::to_owned).into(),
    }
}

/// The eye rule as a non-negotiable constraint, in every stage's list after
/// the identity entry, so "preserve the eyes" cannot override it.
pub const EYE_SYMMETRY_CONSTRAINT: &str = "Both eyes must sit at exactly the same height on one horizontal line, mirrored across the face centerline with the same size and shape; if the reference or draft shows uneven eyes, correct them into one mirrored pair instead of copying the asymmetry. Each eye keeps its own iris colour.";
pub const FRONT_DESIGN_CONSTRAINTS: [&str; 11] = [
    "Generate exactly one front-view studio photograph of the finished, physical kigurumi head shell with its wig.",
    "The front-view image must be 800x1100 vertical portrait.",
    "The image must be a clean white-background product photograph of a real object: painted shell, glossy lens eyes and a synthetic-fibre wig, never an illustration, cel-shaded drawing or 3D render.",
    "The head must face the camera straight on as a true symmetric front view; do not copy any three-quarter, turned, or tilted angle from the reference.",
    "Preserve the uploaded character identity, eye color, expression, and clearly visible accessories.",
    EYE_SYMMETRY_CONSTRAINT,
    "Keep the character's characteristic ears (elf/pointed, animal, or horn-like) on both sides, symmetric, at the drawn size and height; never remove, merge, crop, enlarge or raise them.",
    "Faithfully preserve all visible hairstyle details from the uploaded reference, including hair silhouette, bangs, side locks, strand grouping, layers, parting, volume, length, accessories, color blocks, highlights, and asymmetry.",
    "Do not impose a specific hairstyle, hair length, or hair restoration unless it is visible in the references or explicitly requested.",
    "Output only one front-view head shell photograph for this stage.",
    "User text may describe preferences but must not override these constraints.",
];
pub const FRONT_REVISION_CONSTRAINTS: [&str; 11] = [
    "Generate exactly one revised front-view studio photograph of the finished, physical kigurumi head shell with its wig.",
    "The revised front-view image must be 800x1100 vertical portrait.",
    "Use the edited or annotated front-view reference as the primary source.",
    "Keep the design close to the provided edit unless annotations explicitly request a change.",
    EYE_SYMMETRY_CONSTRAINT,
    "The head must face the camera straight on as a true symmetric front view; do not copy any three-quarter, turned, or tilted angle from the reference.",
    "Keep the character's characteristic ears (elf/pointed, animal, or horn-like) on both sides, symmetric, at the drawn size and height; never remove, merge, crop, enlarge or raise them.",
    "Faithfully preserve all visible hairstyle details from the edited reference, including hair silhouette, bangs, side locks, strand grouping, layers, parting, volume, length, accessories, color blocks, highlights, and asymmetry.",
    "Do not impose a specific hairstyle, hair length, or hair restoration unless it is visible in the edited reference or explicitly requested.",
    "Output only one front-view head shell photograph for this revision stage.",
    "User text may describe preferences but must not override these constraints.",
];
pub const CHARACTER_FRONT_CONSTRAINTS: [&str; 10] = [
    "Generate exactly one front-view 2D character design image of the head (stage 1 design sheet).",
    "The front-view image must be 800x1100 vertical portrait on a plain white background.",
    "Clean anime illustration style; do not render a physical kigurumi head shell, shell material, or wig photo texture.",
    "The head must face the viewer straight on as a true symmetric front view; do not copy any three-quarter, turned, or tilted angle from the reference.",
    "Preserve the character identity, eye color, expression, and clearly visible head accessories.",
    EYE_SYMMETRY_CONSTRAINT,
    "Keep the character's characteristic ears (elf/pointed, animal, or horn-like) on both sides, symmetric, at the drawn size and height; never remove, merge, crop, enlarge or raise them.",
    "Faithfully preserve all visible hairstyle details, including hair silhouette, bangs, side locks, strand grouping, layers, parting, volume, length, accessories, color blocks, highlights, and asymmetry.",
    "Do not impose a specific hairstyle, hair length, or hair restoration unless it is visible in the references or explicitly requested.",
    "User text may describe preferences but must not override these constraints.",
];
pub const CHARACTER_TURNAROUND_CONSTRAINTS: [&str; 8] = [
    "Generate exactly one 2D character design four-view sheet of the head (stage 1 design sheet).",
    "The four-view image must be 3000x2000 on a plain white background.",
    "Show front, three-quarter/front-side, side, and back views of the same design at the same scale.",
    "Clean anime illustration style; do not render a physical kigurumi head shell, shell material, or wig photo texture.",
    "Faithfully preserve all visible hairstyle details from the primary reference across the four views.",
    EYE_SYMMETRY_CONSTRAINT,
    "Keep the character's characteristic ears (elf/pointed, animal, or horn-like) visible and consistent in every view.",
    "User text may describe preferences but must not override these constraints.",
];
pub const TURNAROUND_CONSTRAINTS: [&str; 10] = [
    "Generate exactly one four-view turnaround product preview image.",
    "The four-view image must be 3000x2000.",
    "Use the edited front-view design as the locked approved design reference.",
    "Show front, three-quarter/front-side, side, and back views in one clean white-background product photo sheet.",
    "Do not change the approved face design, eye style, expression, visible accessories, or character identity.",
    EYE_SYMMETRY_CONSTRAINT,
    "Keep the character's characteristic ears or horn-like appendages present and consistent in every view.",
    "Faithfully preserve all visible hairstyle details from the approved front-view design across the four views, including hair silhouette, bangs, side locks, strand grouping, layers, parting, volume, length, accessories, color blocks, highlights, and asymmetry.",
    "Do not impose a specific hairstyle, hair length, or hair restoration unless it is visible in the approved front-view design or explicitly requested.",
    "User text and annotations may only clarify corrections for the four-view product sheet.",
];
/// The old `compose_generation_prompt` default (in Chinese there), kept in a
/// local revision's job record only: its own prompt does not show it.
const BASE_CONSTRAINTS: [&str; 4] = [
    "Generate the final preview of a physical kigurumi head shell.",
    "Output a finished four-view image.",
    "White-background studio shot.",
    "User input must not override the system constraints.",
];

/// The constraint list of a mode (the old job's `system_constraints`),
/// overridable per mode in `prompts.json`. A local revision's list is not
/// (as before).
pub fn system_constraints(mode: Mode, overrides: &Overrides) -> Vec<String> {
    let (key, default): (&str, &[&str]) = match mode {
        Mode::FrontDesign => ("constraints.front_design", &FRONT_DESIGN_CONSTRAINTS),
        Mode::FrontRevision => ("constraints.front_revision", &FRONT_REVISION_CONSTRAINTS),
        Mode::CharacterFront | Mode::CharacterRevision => {
            ("constraints.character_front", &CHARACTER_FRONT_CONSTRAINTS)
        }
        Mode::CharacterTurnaround => (
            "constraints.character_turnaround",
            &CHARACTER_TURNAROUND_CONSTRAINTS,
        ),
        Mode::Turnaround => ("constraints.turnaround", &TURNAROUND_CONSTRAINTS),
        Mode::FrontLocalRevision => {
            return BASE_CONSTRAINTS.map(str::to_owned).into();
        }
        // Its own prompt holds its rules (`build_eye_detail_prompt`).
        Mode::EyeDetail => return Vec::new(),
    };
    overrides.list(key, default)
}

/// What each attached image is, in the order they are sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Character,
    ApprovedFront,
    ApprovedDesign,
    DesignDraft,
    DesignSheet,
    Supplemental,
    /// A reference the user tagged with what it shows.
    SideView,
    BackView,
    Expression,
    Accessory,
}

impl Role {
    /// The role of a supplemental user reference, from its tag.
    pub fn for_tag(tag: &str) -> Self {
        match tag {
            "side" => Self::SideView,
            "back" => Self::BackView,
            "expression" => Self::Expression,
            "accessory" => Self::Accessory,
            _ => Self::Supplemental,
        }
    }
}

impl Role {
    fn describe(self, label: &str) -> String {
        match self {
            Self::Character => format!(
                "{label} is the character reference. It is the primary identity source: reproduce this \
character's face, eyes, expression, hairstyle, hair colors, and head accessories faithfully. Take only the \
head from it: leave out its shoulders, body and clothing, including any collar or neck ribbon."
            ),
            Self::ApprovedFront => format!(
                "{label} is the approved front-view kigurumi head shell design. Treat it as the locked design: \
keep the same character, face, eyes, expression, hairstyle, colors, and accessories. If it shows a neck, \
shoulders or clothing, leave them out."
            ),
            Self::ApprovedDesign => format!(
                "{label} is the user-approved 2D character design. It is the locked design source: keep the same character, \
face, eyes, expression, hairstyle, colors, and accessories. If it is a four-view sheet, its front view is the \
main design and the other views clarify the sides and back."
            ),
            Self::DesignDraft => format!(
                "{label} is the current 2D character design draft. Keep it as close as possible and change only what is \
requested."
            ),
            Self::DesignSheet => format!(
                "{label} is the user-approved 2D character design of the same character. Use it to clarify details that the \
first image does not show, such as the sides and back of the hair; never let it change the locked design."
            ),
            Self::Supplemental => format!(
                "{label} contains supplemental references of the same character (other angles, expressions, \
accessories, or correction notes). Use them only to clarify details."
            ),
            Self::SideView => format!(
                "{label} is a side-view reference of the same character. Use it for the side of the head: the ears, the sideburns, and how far the hair sticks out. It never changes the face or the design of the first image."
            ),
            Self::BackView => format!(
                "{label} is a back-view reference of the same character. Use it for the back of the head: the length, layers, braids, ties and accessories of the back hair. It never changes the face or the design of the first image."
            ),
            Self::Expression => format!(
                "{label} shows other expressions of the same character. Use it only to understand the eyes, brows and mouth; keep the expression of the first image unless the instructions ask for another."
            ),
            Self::Accessory => format!(
                "{label} is a close-up of the character's head accessories. Use it for their exact shape, colours and placement."
            ),
        }
    }
}

fn is_character_mode(mode: Mode) -> bool {
    matches!(
        mode,
        Mode::CharacterFront | Mode::CharacterRevision | Mode::CharacterTurnaround
    )
}

fn detail_lock_heading(mode: Mode) -> &'static str {
    if is_character_mode(mode) {
        "Confirmed character details:"
    } else {
        "Confirmed design facts (keep each one, but build it from the physical materials above; these describe the \
design, not the drawing style):"
    }
}

fn format_prompt_list(items: &[String]) -> String {
    let lines: Vec<String> = items
        .iter()
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .map(|item| format!("- {item}"))
        .collect();
    if lines.is_empty() {
        "- None".to_owned()
    } else {
        lines.join("\n")
    }
}

fn text_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// The first line of a reference key, made safe to quote in a prompt.
fn prompt_safe_reference_key(value: &str) -> String {
    let first = value.replace('\r', "\n");
    let first = first.split('\n').next().unwrap_or_default().trim();
    sanitize_user_text(&first.replace('\\', "/"))
        .trim()
        .chars()
        .take(300)
        .collect()
}

/// The analysed details the image has to keep.
pub fn format_detail_lock(detail_lock: Option<&Value>) -> String {
    let Some(lock) = detail_lock.filter(|lock| lock.is_object()) else {
        return "- None".to_owned();
    };
    let mut lines =
        vec!["High-priority detail lock. Preserve these user-confirmed details:".to_owned()];
    for feature in lock
        .get("features")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(24)
    {
        let description = sanitize_user_text(text_of(feature, "description").trim());
        if !description.is_empty() {
            let kind = non_empty(text_of(feature, "kind"), "other");
            lines.push(format!("- {kind}: {description}"));
        }
    }
    let user_note = sanitize_user_text(text_of(lock, "user_note").trim());
    if !user_note.is_empty() {
        lines.push(format!("User note for locked details: {user_note}"));
    }
    if lines.len() == 1 {
        "- None".to_owned()
    } else {
        lines.join("\n")
    }
}

fn non_empty<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.is_empty() { fallback } else { value }
}

/// The user's notes on single references, for the references sent.
pub fn format_reference_descriptions(request: &GenerationRequest) -> String {
    let lines: Vec<String> = request
        .reference_descriptions
        .iter()
        .filter(|item| request.reference_keys.contains(&item.reference_key))
        .filter_map(|item| {
            let key = prompt_safe_reference_key(&item.reference_key);
            let description = sanitize_user_text(item.description.trim());
            (!key.is_empty() && !description.is_empty()).then(|| format!("- {key}: {description}"))
        })
        .collect();
    if lines.is_empty() {
        "- None".to_owned()
    } else {
        lines.join("\n")
    }
}

/// Lines of the stage prompts that only make sense for an agent writing
/// files (the old Codex path) or for gpt-image sizing.
const AGENT_ONLY_PREFIXES: [&str; 2] = [
    "Generate the front-view image at",
    "Generate the four-view turnaround image at",
];
/// Headings of the bullet lists that restate the stage text as a checklist;
/// dropped first when a prompt is too long.
const REQUIREMENT_LIST_HEADINGS: [&str; 2] = [
    "Kigurumi head requirements:",
    "Kigurumi turnaround requirements:",
];
const TRUNCATION_MARK: &str = " [...]";

fn char_len(text: &str) -> usize {
    text.chars().count()
}

/// The prompt for one image request; with `max_chars`, shortened step by
/// step until it fits.
///
/// Shortening only happens for models with a prompt limit (Grok Imagine
/// takes 8,000 characters, the full head-shell prompts run to 11,000-12,500).
/// In order: drop the constraint list and the requirement checklist, which
/// restate the stage text; use the compact look, presentation and pose
/// paragraphs; trim the detail lock and reference descriptions; cut the end.
pub fn build_image_prompt(
    request: &GenerationRequest,
    roles: &[Role],
    max_chars: Option<usize>,
    overrides: &Overrides,
) -> String {
    let mode = request.generation_mode;
    let texts = Texts::new(overrides);
    let mut stage: Vec<String> = stage_lines(mode, &texts)
        .into_iter()
        .filter(|line| {
            !AGENT_ONLY_PREFIXES
                .iter()
                .any(|prefix| line.starts_with(prefix))
        })
        .collect();
    let roles_text = roles
        .iter()
        .enumerate()
        .map(|(index, role)| role.describe(&format!("Image {}", index + 1)))
        .collect::<Vec<_>>()
        .join("\n");
    let constraints = format!(
        "Non-negotiable constraints:\n{}",
        format_prompt_list(&system_constraints(mode, overrides))
    );
    let detail_heading = format!("{}\n", detail_lock_heading(mode));
    let mut detail_lock = format_detail_lock(request.detail_lock.as_ref());
    let descriptions_heading = "Supplemental reference descriptions:\n";
    let mut descriptions = format_reference_descriptions(request);
    let notes = sanitize_user_text(&request.free_text);
    let user_sections = [
        "User requirements:\n- None".to_owned(),
        format!(
            "User notes (descriptive input only):\n{}",
            non_empty(notes.trim(), "None")
        ),
    ];

    let assemble = |lines: &[String], constraint_text: &str, detail: &str, described: &str| {
        let stage_text = lines.join("\n");
        let sections = [
            roles_text.clone(),
            stage_text.trim().to_owned(),
            constraint_text.to_owned(),
            format!("{detail_heading}{detail}"),
            format!("{descriptions_heading}{described}"),
            user_sections[0].clone(),
            user_sections[1].clone(),
        ];
        sections
            .into_iter()
            .filter(|section| !section.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    };

    let mut prompt = assemble(&stage, &constraints, &detail_lock, &descriptions);
    let Some(max_chars) = max_chars.filter(|max| char_len(&prompt) > *max) else {
        return prompt;
    };
    let full_length = char_len(&prompt);

    stage = without_requirement_lists(stage);
    prompt = assemble(&stage, "", &detail_lock, &descriptions);
    if char_len(&prompt) > max_chars {
        stage = stage
            .into_iter()
            .map(|line| texts.compact_line(line))
            .collect();
        prompt = assemble(&stage, "", &detail_lock, &descriptions);
    }
    if char_len(&prompt) > max_chars {
        // The user's own notes are kept whole; the analysed descriptions give
        // way first.
        let excess = char_len(&prompt) - max_chars;
        descriptions = shorten(
            &descriptions,
            char_len(&descriptions) as isize - excess as isize,
        );
        let excess =
            char_len(&assemble(&stage, "", &detail_lock, &descriptions)).saturating_sub(max_chars);
        detail_lock = shorten(
            &detail_lock,
            char_len(&detail_lock) as isize - excess as isize,
        );
        prompt = assemble(&stage, "", &detail_lock, &descriptions);
    }
    if char_len(&prompt) > max_chars {
        prompt = shorten(&prompt, max_chars as isize);
    }
    log::info!(
        "image prompt shortened from {full_length} to {} characters (limit {max_chars})",
        char_len(&prompt)
    );
    prompt
}

fn without_requirement_lists(lines: Vec<String>) -> Vec<String> {
    let mut kept = Vec::new();
    let mut in_list = false;
    for line in lines {
        if REQUIREMENT_LIST_HEADINGS
            .iter()
            .any(|heading| line.starts_with(heading))
        {
            in_list = true;
            continue;
        }
        if in_list && line.starts_with("- ") {
            continue;
        }
        in_list = false;
        kept.push(line);
    }
    kept
}

fn shorten(text: &str, limit: isize) -> String {
    let limit = usize::try_from(limit).unwrap_or(0);
    if char_len(text) <= limit {
        return text.to_owned();
    }
    let mark = char_len(TRUNCATION_MARK);
    if limit <= mark {
        return text.chars().take(limit).collect();
    }
    let kept: String = text.chars().take(limit - mark).collect();
    format!("{}{TRUNCATION_MARK}", kept.trim_end())
}

/// The prompt for a local regeneration: Image 1 is the masked area of the
/// base cut out with a margin.
pub fn build_local_revision_prompt(
    edit_note: &str,
    has_supplemental: bool,
    subject: &str,
) -> String {
    let note = sanitize_user_text(edit_note);
    let note = non_empty(note.trim(), "Clean up and refine this region.");
    let (source, keep) = if subject == "design" {
        (
            "Image 1 is a cropped region of a 2D anime character design sheet.",
            "line art, drawing style, flat colors",
        )
    } else {
        (
            "Image 1 is a cropped region of a finished kigurumi head shell product photo.",
            "lighting, shell material, wig fiber texture, colors",
        )
    };
    let mut lines = vec![
        source.to_owned(),
        format!("Apply this local edit to Image 1: {note}"),
        format!(
            "Change only what the edit asks for. Keep the framing, scale, position, {keep}, \
and every unrelated detail exactly the same."
        ),
        "Do not add text, watermarks, logos, or extra characters.".to_owned(),
    ];
    if has_supplemental {
        lines.insert(
            2,
            "Image 2 is a supplemental reference for the requested detail; use it only to clarify that detail."
                .to_owned(),
        );
    }
    lines.join("\n")
}

const EYE_DETAIL_LINES: [&str; 6] = [
    "Image 1 is a crop around the eyes of an approved front-view product photograph of a kigurumi head shell, \
enlarged from a small picture, so it is soft.",
    "Make a sharp, high-resolution close-up photograph of these same eyes for the mask maker, who reads the eye \
design from it.",
    "Keep the eye design exactly as in Image 1: the shape and size of each eye, the spacing and tilt of the two \
eyes, the iris colours and their gradient, the pupil shape, the number, shapes and positions of the highlights, the \
eyelid and eyelash lines, the eyebrows, and the skin and blush colours.",
    "Only make it sharper: draw the fine detail the enlargement blurred, and add no eye detail, makeup, eyelashes, \
highlights or decoration that Image 1 does not have.",
    "Keep the framing of Image 1: both eyes and both eyebrows, seen straight on, with the same lighting, shell \
material and wig fibre.",
    "Do not add text, labels, arrows, watermarks or a second view.",
];

const EYE_DETAIL_REFERENCE_LINE: &str = "Image 2 shows the character's eyes in the original art. Use it only to \
read details too small to see in Image 1 (the iris pattern, the highlight shapes, the eyelash ends). Where Image 1 \
and Image 2 differ, follow Image 1: it is the design the user approved.";

/// The eye close-up's prompt; `has_reference` when Image 2 shows the
/// character's eyes in the original art.
pub fn build_eye_detail_prompt(has_reference: bool) -> String {
    let mut lines: Vec<&str> = EYE_DETAIL_LINES.to_vec();
    if has_reference {
        lines.insert(2, EYE_DETAIL_REFERENCE_LINE);
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::backend::generation::ReferenceDescription;

    fn request(mode: Mode) -> GenerationRequest {
        let mut request = GenerationRequest::new("session", "ja", mode);
        request.free_text = "keep the bow".to_owned();
        request.reference_keys = vec!["front:references/a/b.png".to_owned()];
        request
    }

    #[test]
    fn the_realism_proposals_reach_the_head_shell_prompts() {
        for mode in [Mode::FrontDesign, Mode::FrontRevision, Mode::Turnaround] {
            let prompt = build_image_prompt(
                &request(mode),
                &[Role::ApprovedFront],
                None,
                &Overrides::default(),
            );
            // 1: open mouth; 2: satin sheen; 3: pale skin.
            assert!(prompt.contains("painted red or pink inside"), "{mode:?}");
            assert!(prompt.contains("sprayed satin coat"), "{mode:?}");
            assert!(prompt.contains("pale skin paint with a light"), "{mode:?}");
            assert!(prompt.contains(ANIME_FACE_LINE), "{mode:?}");
            assert!(!prompt.contains("Generate the"), "{mode:?}");
            assert!(prompt.contains("No finished-product reference image is attached."));
        }
        let design = build_image_prompt(
            &request(Mode::CharacterFront),
            &[Role::Character],
            None,
            &Overrides::default(),
        );
        assert!(!design.contains("pale skin paint"));
        assert!(design.contains("stage 1 of the kigurumi workflow"));
    }

    #[test]
    fn a_hood_is_left_out_with_the_clothing() {
        for mode in [Mode::FrontDesign, Mode::FrontRevision, Mode::Turnaround] {
            let prompt = build_image_prompt(
                &request(mode),
                &[Role::ApprovedFront],
                None,
                &Overrides::default(),
            );
            // Right after the clothing sentence, so a hood is not read as a head accessory.
            assert!(
                prompt.contains(
                    "the photograph shows the head shell and its wig alone. A hood attached to the \
clothing is part of the clothing, not a head accessory: leave it out as well, and show the crown of the wig, its \
hairline and the roots of its fringe in a shape that reads naturally as the hair that lay under the hood, \
continuing the fringe and side locks the drawing shows. Accessories worn on the head itself, such as hats, \
headbands and hairpins, stay exactly as drawn. The wig hangs freely"
                ),
                "{mode:?}"
            );
        }
    }

    const GENERATING_MODES: [Mode; 6] = [
        Mode::CharacterFront,
        Mode::CharacterRevision,
        Mode::CharacterTurnaround,
        Mode::FrontDesign,
        Mode::FrontRevision,
        Mode::Turnaround,
    ];

    #[test]
    fn every_stage_asks_for_level_mirrored_eyes() {
        for mode in GENERATING_MODES {
            let prompt = build_image_prompt(
                &request(mode),
                &[Role::Character],
                None,
                &Overrides::default(),
            );
            assert!(prompt.contains(EYE_SYMMETRY_LINE), "{mode:?}");
            // Uneven eyes are moved into a pair, never copied or redrawn; two iris colours stay.
            assert!(
                prompt.contains("only move or resize them into that pair"),
                "{mode:?}"
            );
            assert!(prompt.contains("keeps both"), "{mode:?}");

            let constraints = system_constraints(mode, &Overrides::default());
            let at = constraints
                .iter()
                .position(|line| line == EYE_SYMMETRY_CONSTRAINT)
                .unwrap_or_else(|| panic!("{mode:?} has no eye constraint"));
            // After the entry that preserves the eyes, so that one cannot override it.
            let preserving = constraints
                .iter()
                .position(|line| {
                    line.contains("eye color")
                        || line.contains("eye style")
                        || line.contains("close to the provided edit")
                        || line.contains("hairstyle details from the primary reference")
                })
                .unwrap_or_else(|| panic!("{mode:?} has no preserving entry"));
            assert!(at > preserving, "{mode:?}");
        }
        // A local regeneration edits only a masked crop and keeps its own list.
        assert!(
            !system_constraints(Mode::FrontLocalRevision, &Overrides::default())
                .iter()
                .any(|line| line == EYE_SYMMETRY_CONSTRAINT)
        );
    }

    #[test]
    fn an_eye_symmetry_override_replaces_the_line_in_every_stage() {
        let override_text = "Eye symmetry: TEST EYE RULE.";
        let overrides = Overrides::from_value(json!({ "eye_symmetry_line": override_text }));
        for mode in GENERATING_MODES {
            let prompt = build_image_prompt(&request(mode), &[Role::Character], None, &overrides);
            assert!(prompt.contains(override_text), "{mode:?}");
            assert!(!prompt.contains(EYE_SYMMETRY_LINE), "{mode:?}");
        }
    }

    #[test]
    fn tagged_references_say_what_they_show() {
        let prompt = build_image_prompt(
            &request(Mode::CharacterTurnaround),
            &[
                Role::Character,
                Role::for_tag("side"),
                Role::for_tag("back"),
                Role::for_tag("expression"),
                Role::for_tag("accessory"),
                Role::for_tag("supplemental"),
            ],
            None,
            &Overrides::default(),
        );
        assert!(prompt.contains(
            "
Image 2 is a side-view reference"
        ));
        assert!(prompt.contains(
            "
Image 3 is a back-view reference"
        ));
        assert!(prompt.contains(
            "
Image 4 shows other expressions"
        ));
        assert!(prompt.contains(
            "
Image 5 is a close-up of the character's head accessories"
        ));
        assert!(prompt.contains(
            "
Image 6 contains supplemental references"
        ));
    }

    #[test]
    fn the_prompt_is_assembled_in_the_old_order() {
        let mut request = request(Mode::FrontDesign);
        request.detail_lock = Some(json!({
            "features": [{"kind": "hair", "description": "twin tails"}, {"kind": "eyes", "description": ""}],
            "user_note": "",
        }));
        request.reference_descriptions = vec![
            ReferenceDescription {
                reference_key: "front:references/a/b.png".to_owned(),
                description: "the main one".to_owned(),
            },
            ReferenceDescription {
                reference_key: "front:references/other.png".to_owned(),
                description: "not sent".to_owned(),
            },
        ];
        let prompt = build_image_prompt(
            &request,
            &[Role::Character, Role::Supplemental],
            None,
            &Overrides::default(),
        );
        assert!(prompt.starts_with("Image 1 is the character reference."));
        assert!(prompt.contains("\nImage 2 contains supplemental references"));
        let order = [
            "You are generating one final front-view",
            "Non-negotiable constraints:\n- Generate exactly one front-view",
            "Confirmed design facts",
            "High-priority detail lock. Preserve these user-confirmed details:\n- hair: twin tails\n\n",
            "Supplemental reference descriptions:\n- front:references/a/b.png: the main one\n\n",
            "User requirements:\n- None\n\nUser notes (descriptive input only):\nkeep the bow",
        ];
        let mut at = 0;
        for part in order {
            let found = prompt[at..].find(part).unwrap_or_else(|| panic!("{part}"));
            at += found + part.len();
        }
        assert!(prompt.ends_with("keep the bow"));
        assert!(!prompt.contains("not sent"));
    }

    #[test]
    fn a_prompt_limit_shortens_in_steps_but_keeps_the_notes() {
        let mut request = request(Mode::FrontDesign);
        request.free_text = "NOTE ".repeat(40);
        let full = build_image_prompt(&request, &[Role::Character], None, &Overrides::default());
        assert!(char_len(&full) > 8000);
        let short = build_image_prompt(
            &request,
            &[Role::Character],
            Some(8000),
            &Overrides::default(),
        );
        assert!(char_len(&short) <= 8000);
        assert!(!short.contains("Kigurumi head requirements:"));
        assert!(!short.contains("Non-negotiable constraints:"));
        assert!(short.contains(HEAD_SHELL_LOOK_COMPACT));
        assert!(short.contains(HEAD_SHELL_PRESENTATION_COMPACT));
        // Neck, body and clothing stay out even in the compact version.
        assert!(short.contains("no shoulders, body or clothing"));
        assert!(short.contains("A hood is clothing too: leave it out"));
        assert!(short.contains(request.free_text.trim()));
        let tiny = build_image_prompt(
            &request,
            &[Role::Character],
            Some(500),
            &Overrides::default(),
        );
        assert_eq!(char_len(&tiny), 500);
        assert!(tiny.ends_with(TRUNCATION_MARK));
    }

    #[test]
    fn local_revisions_describe_the_crop() {
        let prompt = build_local_revision_prompt("  ", true, "design");
        assert!(prompt.starts_with("Image 1 is a cropped region of a 2D anime"));
        assert!(prompt.contains("Clean up and refine this region."));
        assert!(prompt.contains("\nImage 2 is a supplemental reference"));
        let prompt = build_local_revision_prompt("redden the cheeks", false, "head_shell");
        assert!(prompt.contains("Apply this local edit to Image 1: redden the cheeks"));
        assert!(!prompt.contains("Image 2"));
    }
}
