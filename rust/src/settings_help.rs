//! Searchable setting descriptions shared by the desktop and configuration guide.
pub const GUIDE: &[(&str, &str)] = &[
    (
        "tessdata_dir",
        "Verified optional printed-OCR resource folder. Empty uses bundled/system Tesseract data. Interface language is independent.",
    ),
    (
        "ui.locale",
        "Language of core interface labels: en (English), si (Sinhala), ta (Tamil). Detailed help remains English; OCR language is configured separately.",
    ),
    (
        "preset",
        "Fast: printed OCR. Balanced: AI as needed. Accuracy: independent OCR and vision. Maximum: a third reader family and more checks.",
    ),
    (
        "automatic_models",
        "Choose the strongest installed readers fitting currently available memory. Disable to choose model_ids manually.",
    ),
    (
        "model_ids",
        "Reader IDs to allow when automatic selection is off. Manage readers in Studio.",
    ),
    (
        "handwriting",
        "Extra care for handwriting and difficult scans. Requires AI; switches Fast to Accuracy.",
    ),
    (
        "languages",
        "Installed Tesseract language codes joined with +, for example eng+sin. Matching traineddata files are required.",
    ),
    (
        "page_range",
        "Pages starting at 1. Blank means all; examples: 1-4,8.",
    ),
    (
        "dpi",
        "Page render resolution. 0 is automatic. Higher values retain small writing but cost memory and time.",
    ),
    (
        "max_pixels",
        "Maximum pixels per rendered page, protecting memory on very large scans.",
    ),
    (
        "image_edge",
        "Longest AI image edge. 0 uses the reader recommendation. Smaller values can lose fine writing.",
    ),
    (
        "tile_edge",
        "Pixel size of detailed region crops for re-reading difficult areas.",
    ),
    (
        "tile_overlap",
        "Crop overlap as a fraction, helping avoid words cut at boundaries.",
    ),
    (
        "crop_retries",
        "Maximum extra region readings per page. More retries cost additional time.",
    ),
    (
        "consensus_min",
        "Minimum independent reader evidence for agreement. Repeated output from one family is not independent.",
    ),
    (
        "agreement_threshold",
        "Required similarity between independent readings. Conflicting numbers stay as observed alternatives; never averaged.",
    ),
    (
        "review_threshold",
        "Mark low-confidence regions for review. Model confidence is not a calibrated accuracy probability.",
    ),
    (
        "backend",
        "Auto selects compatible CUDA, Vulkan or CPU. GPU presence alone does not prove accelerated inference.",
    ),
    (
        "gpu_index",
        "Optional device index. Null selects the GPU with most available memory.",
    ),
    (
        "ram_fraction",
        "Fraction of free system RAM available for recognition, from 0 to 1.",
    ),
    (
        "vram_fraction",
        "Fraction of free GPU memory available for recognition. Leave headroom for other apps.",
    ),
    (
        "reserve_ram_mib",
        "Extra system memory to keep free in MiB. 1024 MiB equals 1 GiB.",
    ),
    ("reserve_vram_mib", "Extra GPU memory to keep free, in MiB."),
    (
        "cpu_threads",
        "Inference workers. 0 is automatic. Optimize for this PC measures candidate counts.",
    ),
    (
        "context",
        "Model context token budget. 0 is adaptive. Larger contexts use more memory.",
    ),
    (
        "max_tokens",
        "Maximum answer tokens per request. Truncated results are rejected; increase for dense pages.",
    ),
    (
        "batch",
        "Prompt processing batch size. 0 is automatic. Large values can use more memory.",
    ),
    (
        "micro_batch",
        "Physical inference micro-batch size. 0 is automatic; reduce for memory pressure.",
    ),
    (
        "image_tokens",
        "Image token budget for compatible readers. 0 is automatic.",
    ),
    (
        "gpu_layers",
        "Layers to offload. -1 requests full offload. Actual offload is checked in native logs.",
    ),
    (
        "flash_attention",
        "Use memory-efficient attention when the runtime supports it.",
    ),
    (
        "projector_on_cpu",
        "Encode images on CPU to save VRAM; this may be slower.",
    ),
    (
        "kv_cache",
        "Attention-cache precision. q8_0 balances memory and fidelity; support depends on the runtime.",
    ),
    (
        "temperature",
        "Sampling randomness. Lower values make transcription more repeatable.",
    ),
    (
        "top_p",
        "Sampling probability cutoff. Usually keep the default for faithful reading.",
    ),
    (
        "seed",
        "Sampling seed. Different hardware can still produce different results.",
    ),
    (
        "request_timeout_secs",
        "Time limit for one reader request. Slow CPU inference may need more time.",
    ),
    (
        "allow_cpu_offload",
        "Allow readers to spill into system RAM beyond VRAM. This can be much slower.",
    ),
    (
        "require_acceleration",
        "Reject inference unless the managed runtime confirms GPU layer offload.",
    ),
    (
        "tesseract",
        "Printed OCR executable path. Windows installers include English language data.",
    ),
    (
        "pdfium",
        "Path or folder of the compatible PDF rendering library, included in installers.",
    ),
    (
        "server",
        "Native llama-server executable. Verified setup and runtime updates manage this path.",
    ),
    (
        "models_dir",
        "Folder holding local reader registrations and versioned weights. Requires sufficient disk space.",
    ),
    (
        "endpoint",
        "Literal localhost endpoint for an external inference server. Remote conversion endpoints are blocked.",
    ),
    (
        "external_server",
        "Use a separately managed local inference server instead of launching the bundled runtime.",
    ),
    (
        "subject",
        "Background context for interpretation. Context cannot justify inventing unreadable words.",
    ),
    (
        "writer",
        "Confirmed local handwriting examples and profile, managed in Studio > Writers.",
    ),
    (
        "writer.root",
        "Folder for writer profiles and confirmed examples, retained as user data.",
    ),
    ("writer.id", "Selected writer ID; null means no profile."),
    (
        "writer.use_examples",
        "Supply relevant confirmed examples to compatible readers. This does not retrain model weights.",
    ),
    (
        "assistance",
        "Questions and background references used during recognition.",
    ),
    (
        "assistance.ask_questions",
        "Pause for useful clarification. Questions may be skipped; uncertain text stays marked for review.",
    ),
    (
        "assistance.max_questions",
        "Maximum questions per batch, up to 32.",
    ),
    (
        "assistance.web_lookup",
        "Allow explicit public-topic lookup. Scans are never sent to public search.",
    ),
    (
        "assistance.glossary",
        "Confirmed subject terms, used as background rather than transcription evidence.",
    ),
    (
        "assistance.sources",
        "Public reference excerpts chosen for context, not transcription evidence.",
    ),
    ("deskew", "Straighten tilted scans before reading."),
    (
        "auto_rotate",
        "Detect page orientation. Disable when scans are already correctly oriented.",
    ),
    (
        "preserve_figures",
        "Keep source diagram crops in Word beside their descriptions.",
    ),
    (
        "keep_page_breaks",
        "Keep original page boundaries in exported Word documents.",
    ),
    (
        "font",
        "Font for editable Word text. Writer font packs are exported separately.",
    ),
    ("font_size", "Editable Word text size in points."),
    (
        "ui",
        "Appearance preferences do not invalidate recognition checkpoints.",
    ),
    ("ui.dark", "Use the dark workspace palette."),
    (
        "ui.accent",
        "RGB highlight color, components from 0 to 255.",
    ),
    ("ui.scale", "Scale controls and text for readability."),
    (
        "ui.motion",
        "Overall animation intensity. 0 disables movement.",
    ),
    ("ui.scan_animation", "Animate the active scan area."),
    ("ui.intro_animation", "Animate the workspace on opening."),
    (
        "ui.hover_effects",
        "Highlight interactive controls on hover.",
    ),
    (
        "ui.transition_ms",
        "Interface transition duration in milliseconds.",
    ),
    (
        "ui.ambient_density",
        "Amount of decorative background motion.",
    ),
    (
        "ui.idle_fps",
        "Refresh limit while idle. Lower values use fewer resources.",
    ),
    (
        "ui.reduced_motion",
        "Reduce nonessential movement for comfort and resource use.",
    ),
    ("ui.particles", "Show decorative background particles."),
    ("ui.glow", "Show accent glow effects."),
    ("ui.rounding", "Corner radius of interface surfaces."),
    ("ui.fps", "Maximum animation refresh rate while active."),
    ("ui.compact", "Use less workspace spacing."),
    (
        "ui.background",
        "RGB background color, components from 0 to 255.",
    ),
    (
        "ui.autosave",
        "Save desktop settings and queue between sessions. Conversion checkpoints save independently.",
    ),
    (
        "experience",
        "Live-view and update-check preferences do not affect recognition results.",
    ),
    (
        "experience.live_preview",
        "Show the source region and generated draft answer. Drafts may change; private reasoning channels are never shown.",
    ),
    (
        "experience.check_updates",
        "Check official publishers at most daily when idle. Downloads need your click; no document content is sent.",
    ),
    (
        "experience.update_channel",
        "App update channel: stable excludes preview releases; preview explicitly opts in to newer prereleases. Runtime and reader publisher channels are unchanged. Switching channels never installs or downgrades the app.",
    ),
    (
        "experience.defer_updates_until",
        "Optional UTC time until which automatic publisher checks are deferred. Check all updates now bypasses this deferral. It does not schedule downloads or change recognition checkpoints.",
    ),
];
pub fn description(key: &str) -> &'static str {
    GUIDE
        .iter()
        .find(|(k, _)| *k == key)
        .map_or("", |(_, v)| *v)
}
