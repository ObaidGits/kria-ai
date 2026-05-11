use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;

/// Intent classification result.
#[derive(Debug, Clone)]
pub struct IntentResult {
    pub intent: Intent,
    pub tool_hint: Option<String>,
    pub category: Option<String>,
    pub confidence: f32,
}

/// High-level intent categories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    /// Conversational — no tool needed, direct LLM response.
    Conversation,
    /// Direct tool call — a single tool mapped from user text.
    DirectTool(String),
    /// Complex task — requires planning + multi-step tool use.
    ComplexTask,
}

// ─── Conversation patterns (no tool needed) ───
static CONVERSATION_RE: Lazy<Vec<Regex>> = Lazy::new(|| {
    let patterns = [
        r"^(hi|hello|hey|good\s*(morning|afternoon|evening)|howdy|greetings)\b",
        r"^(who|what)\s+(are|is)\s+you",
        r"^(thank|thanks|thx)\b",
        r"^(bye|goodbye|see\s+you|goodnight)\b",
        r"^(tell\s+me\s+a\s+joke|joke\b)",
        r"^(how\s+are\s+you|what'?s\s+up)\b",
        r"^(explain|describe|what\s+is|what\s+are|define)\b",
    ];
    patterns
        .iter()
        .filter_map(|p| Regex::new(&format!("(?i){p}")).ok())
        .collect()
});

// ─── Direct tool patterns (trigger specific tools) ───
static DIRECT_TOOL_RE: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
    let mappings: Vec<(&str, &str)> = vec![
        // System stats / health (multi-metric — maps to check_system_health as entry point)
        (
            r"(?i)\b(system\s+stat(s|us)|my\s+system\s+stat|mera\s+system|system\s+vitals?)\b",
            "check_system_health",
        ),
        (
            r"(?i)\b(system\s+health|health\s+check)\b",
            "check_system_health",
        ),
        // Installed apps/packages — MUST come before generic search/news patterns
        (
            r"(?i)\b(list|show|get|what|display|view)\b.{0,30}\b(installed)\b.{0,20}\b(apps?|applications?|packages?|programs?|software)\b",
            "list_installed_packages",
        ),
        (
            r"(?i)\b(installed)\b.{0,20}\b(apps?|applications?|packages?|programs?|software)\b",
            "list_installed_packages",
        ),
        (
            r"(?i)\b(apps?|applications?|packages?|programs?|software)\b.{0,20}\b(installed)\b",
            "list_installed_packages",
        ),
        // Alerts
        (
            r"(?i)\b(show|list|get|check|current|active)\b.{0,30}\balerts?\b",
            "get_alerts",
        ),
        (
            r"(?i)\balerts?\b.{0,20}\b(show|list|active|current)\b",
            "get_alerts",
        ),
        (r"(?i)\bdismiss\b.{0,20}\balert\b", "dismiss_alert"),
        // Power plan
        (r"(?i)\bset\b.{0,20}\bpower\s+plan\b", "set_power_plan"),
        (
            r"(?i)\bpower\s+plan\b.{0,20}\b(set|change|switch|to)\b",
            "set_power_plan",
        ),
        (
            r"(?i)\b(current|get|what|show).{0,20}\bpower\s+plan\b",
            "get_power_plan",
        ),
        (r"(?i)\bpower\s+plan\b", "get_power_plan"),
        // WiFi networks list
        (
            r"(?i)\b(list|show|available|nearby|scan)\b.{0,20}\b(wifi|wi-fi|wireless)\s*(networks?|ssid|connections?)\b",
            "get_wifi_networks",
        ),
        // Active window / window management
        (
            r"(?i)\b(active|current|focused)\b.{0,15}\bwindow\b|\bwindow.{0,15}\b(active|current|focused)\b",
            "get_active_window",
        ),
        (
            r"(?i)\b(list|show|all)\b.{0,15}\b(open\s+windows?|windows?)\b",
            "list_windows",
        ),
        // Active network connections
        (
            r"(?i)\b(active|open|current)\b.{0,20}\b(network\s+connections?|connections?|sockets?)\b",
            "get_active_connections",
        ),
        // Service management — routed to execute_bash for command-level granularity
        (
            r"(?i)\b(start|stop|restart|status|check)\b.{0,20}\b(service|daemon|systemd)\b",
            "execute_bash",
        ),
        // Scheduled tasks
        (
            r"(?i)\b(list|show|my)\b.{0,20}\b(scheduled\s+tasks?|cron\s+jobs?|timers?)\b",
            "list_scheduled_tasks",
        ),
        // System info
        (
            r"(?i)\b(cpu|processor)\s*(usage|load|info|stats?|stat)\b",
            "get_cpu_usage",
        ),
        (
            r"(?i)\bmy\s+cpu\b|\bcpu\s+ka\s+(use|usage|haal)\b",
            "get_cpu_usage",
        ),
        (
            r"(?i)\b(ram|memory)\s*(usage|info|status|stats?|stat)\b",
            "get_memory_info",
        ),
        (
            r"(?i)\b(disk|storage)\s*(space|usage|info)\b",
            "get_disk_space",
        ),
        (
            r"(?i)\bcheck\s+(my\s+)?battery\b|\bbattery\s+(check|level|percent|info|status|kya|hai)\b",
            "get_battery_status",
        ),
        (
            r"(?i)\b(battery)\s*(status|level|info)\b",
            "get_battery_status",
        ),
        (
            r"(?i)\b(gpu|graphics)\s*(info|status|usage)\b",
            "get_gpu_info",
        ),
        (r"(?i)\b(uptime|how\s+long.*running)\b", "get_system_uptime"),
        (
            r"(?i)\b(network|internet)\s*(status|info|connection)\b",
            "get_network_status",
        ),
        // App lifecycle — specific patterns first, generic fallback last
        //
        // browser_search: "open Chrome and search X", "search for X on YouTube",
        //                 "play X on YouTube", "google X", "youtube search X"
        (
            r"(?i)\b(open|launch)\s+\w+\s+(and\s+)?(search|google|look\s*up|find)\b",
            "browser_search",
        ),
        (
            r"(?i)\b(search|google|look\s*up)\b.*\b(on\s+)?(youtube|chrome|firefox|browser|web)\b",
            "browser_search",
        ),
        (
            r"(?i)\b(youtube|yt)\s+(search|play|find|look\s*up)\b",
            "browser_search",
        ),
        (
            r"(?i)\b(play|search)\b.{0,40}\b(on|in|via)\s+(youtube|yt)\b",
            "browser_search",
        ),
        // Embeddings (sidecar) — MUST come before send_message so "make text embeddings"
        // is not misclassified as "text <recipient>".
        (
            r"(?i)\b(generate|create|make|compute|get)\s+(text\s+)?embeddings?\b|\bembedding\s+for\b",
            "embeddings_generate",
        ),
        // send_message: "text/message/WhatsApp/signal Anjali", "send a WhatsApp to X"
        // Excludes "text embeddings" / "text message" via the embeddings rule above.
        (r"(?i)\b(text|message|msg)\s+\w+\b", "send_message"),
        (
            r"(?i)\b(send|open)\s+(a\s+)?(whatsapp|telegram|signal)\b",
            "send_message",
        ),
        (
            r"(?i)\b(whatsapp|telegram|signal)\s+(message|msg|text)?\s*(to\s+)?\w+\b",
            "send_message",
        ),
        (
            r"(?i)\bsend\s+(a\s+)?message\s+(to\s+)?\w+\b",
            "send_message",
        ),
        // open_url: "open https://...", "go to <url>"
        (
            r"(?i)\b(open|go\s+to|navigate\s+to|visit)\s+https?://\S+",
            "open_url",
        ),
        // Remote VM / connected target command execution
        (
            r"(?i)\b(?:run|execute|install|uninstall|update|upgrade)\b.{0,80}\b(?:on|in)\s+(?:my\s+)?(?:vm|remote\s+(?:vm|host|machine|computer)|connected\s+(?:vm|computer|machine|host))\b",
            "execute_fleet_command",
        ),
        (
            r"(?i)\bremote\s+command\s*:\s*.+",
            "execute_fleet_command",
        ),
        // Extended VM patterns — catch general VM-related queries
        (
            r"(?i)\bmy\s+(vm|virtual\s+machine|server)\s+(is|seems|looks|running|has|was|did)\b",
            "execute_fleet_command",
        ),
        (
            r"(?i)\b(vm|virtual\s+machine)\s+(task|install|update|upgrade|running|slow|broken|fix|fail|error|problem)\b",
            "execute_fleet_command",
        ),
        (
            r"(?i)\b(stop|check|restart|fix|diagnose)\b.{0,40}\b(vm|virtual\s+machine)\s+(task|process|job)\b",
            "execute_fleet_command",
        ),
        (
            r"(?i)\b(check|verify)\b.{0,30}\b(docker|service|process|disk|cpu|memory|ram)\b.{0,30}\b(on|in)\s+(all\s+)?my\s+(vm|vms|servers?|machines?)\b",
            "execute_fleet_command",
        ),
        (
            r"(?i)\b(why|what|how)\s+(did|does|is|was)\b.{0,40}\b(vm|virtual\s+machine)\b",
            "execute_fleet_command",
        ),
        (
            r"(?i)\b(vm|virtual\s+machine)\b.{0,20}\b(fail|error|crash|broken|slow|issue|problem)\b",
            "execute_fleet_command",
        ),
        (
            r"(?i)\b(?:is\s+it\s+(?:active|up)|check\s+status|status\s+check|is\s+(?:my\s+)?(?:vm|server|vm\d+)\s+up)\b",
            "check_device_health",
        ),
        (
            r"(?i)\b(?:health|heartbeat|reachability)\b.{0,30}\b(?:my\s+vm|server|vm\d+|remote\s+host)\b",
            "check_device_health",
        ),
        (
            r"(?i)\bvia\s+ssh\b|\bssh\s+[a-z0-9_.-]+@[a-z0-9_.:-]+\b",
            "execute_fleet_command",
        ),
        // Shell execution — MUST come before open_application so "Run bash:" is not misclassified
        (
            r"(?i)^run\s*:\s*\S+",
            "execute_bash",
        ),
        (
            r"(?i)\brun\s+(bash|shell|command)\s*:\s*.+",
            "execute_bash",
        ),
        (
            r"(?i)\brun\s+python\s*:\s*.+",
            "execute_python",
        ),
        (
            r"(?i)\brun\s+powershell\s*(command)?\s*:\s*.+",
            "execute_powershell",
        ),
        // Speed test — MUST come before open_application so "Run a speed test" is not misclassified
        (r"(?i)\bspeed\s*test\b", "speed_test"),
        // open_application: generic — last resort for "open/launch/start <app>"
        (
            r"(?i)\b(open|launch|start|run)\s+(\w+)\b",
            "open_application",
        ),
        (r"(?i)\b(close|quit|exit)\s+(\w+)\b", "close_application"),
        (
            r"(?i)\b(running|active)\s*(apps|applications|processes)\b",
            "list_running_apps",
        ),
        (r"(?i)\b(kill|terminate)\s*(process|pid)\b", "kill_process"),
        // Google Workspace (Drive)
        (
            r"(?i)\b(list|show|browse|what'?s\s+in|what\s+is\s+in|contents?)\b.*\b(google\s+drive|drive\s+files?|drive)\b",
            "gw_drive_list",
        ),
        (
            r"(?i)\b(search|find|look\s*for|locate)\b.*\b(google\s+drive|drive\s+files?|drive)\b",
            "gw_drive_search",
        ),
        (
            r"(?i)\b(read|open|view|download|fetch)\b.*\b(google\s+drive|drive)\b.*\b(file|document|doc|spreadsheet|sheet|slides?|presentation)\b",
            "gw_drive_read",
        ),
        (
            r"(?i)\b(read|open|view|download|fetch)\b.*\b(file|document|doc|spreadsheet|sheet|slides?|presentation)\b.*\b(google\s+drive|drive)\b",
            "gw_drive_read",
        ),
        (
            r"(?i)\b(delete|remove|trash)\b.*\b(google\s+drive|drive)\b.*\b(file|document|doc|spreadsheet|sheet|slides?|presentation)\b",
            "gw_drive_delete",
        ),
        (
            r"(?i)\b(delete|remove|trash)\b.*\b(file|document|doc|spreadsheet|sheet|slides?|presentation)\b.*\b(google\s+drive|drive)\b",
            "gw_drive_delete",
        ),
        (
            r"(?i)\b(today'?s?|todays)\b.*\b(google\s+calendar|calendar|schedule|events?)\b|\b(google\s+calendar|calendar|schedule|events?)\b.*\b(today'?s?|todays)\b",
            "gw_calendar_today",
        ),
        (
            r"(?i)\b(latest|recent|current|updates?)\b.*\b(google\s+calendar|calendar|schedule|events?)\b",
            "gw_calendar_search",
        ),
        // MCP filesystem server — explicit "via MCP" / "using the filesystem MCP" routing.
        // These MUST come before generic file ops so MCP-prefixed tools are preferred.
        (
            r"(?i)\b(list|ls|dir)\b.*\b(files?|directory|folder)\b.*\b(mcp|filesystem\s+mcp)\b",
            "mcp_fs_list_directory",
        ),
        (
            r"(?i)\b(mcp|filesystem\s+mcp)\b.*\b(list|ls|dir)\b.*\b(files?|directory|folder)\b",
            "mcp_fs_list_directory",
        ),
        (
            r"(?i)\b(read|show|cat|display|open)\b.*\b(mcp|filesystem\s+mcp)\b",
            "mcp_fs_read_file",
        ),
        (
            r"(?i)\b(mcp|filesystem\s+mcp)\b.*\b(read|show|cat|display|open)\b",
            "mcp_fs_read_file",
        ),
        (
            r"(?i)\b(search|find|grep)\b.*\b(files?|directory|folder)\b.*\b(mcp|filesystem\s+mcp)\b",
            "mcp_fs_search_files",
        ),
        (
            r"(?i)\b(mcp|filesystem\s+mcp)\b.*\b(search|find|grep)\b.*\b(files?|directory|folder)\b",
            "mcp_fs_search_files",
        ),
        // File ops
        (
            r"(?i)\b(read|show|cat|display)\s+(the\s+)?file\b",
            "read_file",
        ),
        (
            r"(?i)\b(read|show|cat|display|open)\s+(/|~/)\S+",
            "read_file",
        ),
        (
            r"(?i)\b(list|ls|dir)\s+(the\s+)?(directories|folders|files)\b",
            "list_directory",
        ),
        (
            r"(?i)\b(list|ls|dir)\s+(the\s+)?(directory|folder|files)\b",
            "list_directory",
        ),
        (r"(?i)\b(search|find)\s+(for\s+)?files?\b", "search_files"),
        (
            r"(?i)\b(search|find|locate|look\s*for)\b.*\b(files?|folder|directory|directories|folders)\b",
            "search_files",
        ),
        (
            r"(?i)\b(files?|folder|directory)\b.*\b(named|called|name)\b",
            "search_files",
        ),
        // "search for foo.txt" / "find bar.pdf" — filename with extension implies file search
        (
            r#"(?i)\b(search|find|locate|look\s*for)\b\s+(for\s+)?["']?[\w\-./]+\.(txt|md|pdf|docx?|xlsx?|csv|json|ya?ml|toml|rs|py|js|ts|tsx|jsx|html|css|png|jpg|jpeg|gif|svg|mp3|mp4|wav|zip|tar|gz)["']?"#,
            "search_files",
        ),
        (r"(?i)\b(write|create|save)\s+(a\s+)?file\b", "write_file"),
        (r"(?i)\b(delete|remove|rm)\s+(the\s+)?file\b", "delete_file"),
        // Clipboard (set rule must run before generic get rule)
        (
            r"(?i)\b(copy|set)\b.{0,24}\bclipboard\b|\bclipboard\b.{0,12}\b(to|with)\b",
            "set_clipboard",
        ),
        (
            r"(?i)\b(get|show|read|paste)\b.{0,24}\bclipboard\b|\bwhat.*copied\b|\bclipboard\b",
            "get_clipboard",
        ),
        (r"(?i)\bscreenshot\b", "screenshot"),
        // Power
        (
            r"(?i)\b(shutdown|shut\s+down|power\s+off)\b",
            "shutdown_system",
        ),
        (
            r"(?i)\b(reboot|restart)\s*(system|computer|pc)?\b",
            "reboot_system",
        ),
        (r"(?i)\block\s*(screen|computer)\b", "lock_screen"),
        (r"(?i)\b(sleep|suspend)\s*(mode|computer)?\b", "sleep"),
        // System config — volume
        (
            r"(?i)\b(volume|sound)\s*(set|to|at)\s*(\d+)\b",
            "set_volume",
        ),
        (
            r"(?i)\b(set|change|put|increase|decrease|raise|lower|turn\s+up|turn\s+down)\b.{0,20}\b(volume|sound|speaker)\b",
            "set_volume",
        ),
        (
            r"(?i)\b(volume|sound|speaker|awaaz)\s+(ko|set|badhao|ghataao|ghatao|badha|ghata|barhao|badhaao)\b|\b(volume|sound|speaker|awaaz)\s+\d+",
            "set_volume",
        ),
        // System config — brightness
        (
            r"(?i)\b(brightness)\s*(set|to|at)\s*(\d+)\b",
            "set_brightness",
        ),
        (
            r"(?i)\b(set|change|increase|decrease|raise|lower|turn\s+up|turn\s+down)\b.{0,20}\bbrightness\b",
            "set_brightness",
        ),
        (
            r"(?i)\bbrightness\s+(ko|set|badhao|ghataao|ghatao|badha|ghata|barhao|badhaao)\b|\bbrightness\s+\d+",
            "set_brightness",
        ),
        (
            r"(?i)\b(wifi)\s*(on|off|enable|disable|toggle)\b",
            "toggle_wifi",
        ),
        // Internet
        (
            r"(?i)\b(latest|breaking|today|current|recent)\b.*\b(news|headlines|updates?)\b",
            "search_news",
        ),
        (
            r"(?i)\b(news|headlines|updates?)\b.*\b(india|indian|pakistan|bangladesh|sri\s*lanka|us|uk|europe|asia|middle\s*east)\b",
            "search_news",
        ),
        (
            r"(?i)\b(news|headlines|updates?)\b.*\b(authentic|trusted|reliable|verified)\b",
            "search_news",
        ),
        (
            r"(?i)\b(search|google|look\s+up|find\s+online)\b.*\b(web|online|internet)\b",
            "web_search",
        ),
        (
            r"(?i)\b(search|google|look\s+up)\s+(for|the|about)\b",
            "web_search",
        ),
        // Google Workspace (Gmail)
        (
            r"(?i)\b(read|open|view|show)\b.*\b(gmail|gmails|email|emails?|mail)\b.*\b(message\s*id|message_id|id)\b",
            "gw_gmail_read",
        ),
        (
            r"(?i)\b(delete|remove|trash)\b.*\b(gmail|gmails|email|emails?|mail)\b",
            "gw_gmail_delete",
        ),
        (
            r"(?i)\b(check|show|list|get|read|fetch)\b.*\b(gmail|gmails|inbox|emails?|mailbox)\b",
            "gw_gmail_inbox",
        ),
        (
            r"(?i)\b(gmail|gmails|inbox|emails?|mailbox)\b.*\b(check|show|list|get|read|fetch|recent|latest|unread)\b",
            "gw_gmail_inbox",
        ),
        (
            r"(?i)\b(unread|recent|latest)\s+(gmail|gmails|emails?)\b",
            "gw_gmail_inbox",
        ),
        (
            r"(?i)\b(search|find|look\s*for)\b.*\b(gmail|gmails|emails?|inbox)\b",
            "gw_gmail_search",
        ),
        (
            r#"(?i)\b(send|write|compose|draft)\b\s+(?:an?\s+|the\s+)?(.+?)\s+\b(mail|email|gmail)\b.*\bto\s+["']?[a-z0-9._%+\-]+@[a-z0-9.\-]+\.[a-z]{2,}["']?"#,
            "gw_gmail_send",
        ),
        // Google Workspace (Calendar / Meet fallback via Calendar)
        (
            r"(?i)\b(what'?s|show|list|check|get|view)\b.*\b(calendar|schedule|events?)\b",
            "gw_calendar_search",
        ),
        (
            r"(?i)\b(today)\b.*\b(meetings?|events?)\b|\b(meetings?|events?)\b.*\b(today)\b",
            "gw_calendar_today",
        ),
        (
            r"(?i)\b(schedule|create|book|add|plan)\b.*\b(calendar\s+event|event|meeting|appointment|meet|call|invite)\b",
            "gw_calendar_create",
        ),
        (
            r"(?i)\b(delete|remove|cancel)\b.*\b(calendar\s+event|event|meeting|appointment)\b",
            "gw_calendar_delete",
        ),
        // Google Workspace (Docs)
        (
            r"(?i)\b(create|new|start|draft|write)\b.*\b(google\s+docs?|gdocs?|gdoc|document)\b",
            "gw_docs_create",
        ),
        (
            r"(?i)\b(read|open|show|view|summarize|extract)\b.*\b(google\s+docs?|gdocs?|gdoc|document)\b",
            "gw_docs_read",
        ),
        (
            r"(?i)\b(edit|update|append|modify)\b.*\b(google\s+docs?|gdocs?|gdoc|document)\b",
            "gw_docs_edit",
        ),
        (
            r"(?i)\b(delete|remove|trash)\b.*\b(google\s+docs?|gdocs?|gdoc|document)\b",
            "gw_drive_delete",
        ),
        // Google Workspace (Sheets)
        (
            r"(?i)\b(create|new|start|make)\b.*\b(google\s+sheets?|gsheets?|spreadsheet|sheet)\b",
            "gw_sheets_create",
        ),
        (
            r"(?i)\b(read|open|show|view|analyze)\b.*\b(google\s+sheets?|gsheets?|spreadsheet|sheet)\b",
            "gw_sheets_read",
        ),
        (
            r"(?i)\b(edit|update|write|append|modify)\b.*\b(google\s+sheets?|gsheets?|spreadsheet|sheet)\b",
            "gw_sheets_edit",
        ),
        (
            r"(?i)\b(delete|remove|trash)\b.*\b(google\s+sheets?|gsheets?|spreadsheet|sheet)\b",
            "gw_drive_delete",
        ),
        // Google Workspace (Slides)
        (
            r"(?i)\b(create|new|start|make)\b.*\b(google\s+slides?|gslides?|presentation|deck)\b",
            "gw_slides_create",
        ),
        (
            r"(?i)\b(read|open|show|view)\b.*\b(google\s+slides?|gslides?|presentation|deck)\b",
            "gw_slides_read",
        ),
        (
            r"(?i)\b(delete|remove|trash)\b.*\b(google\s+slides?|gslides?|presentation|deck)\b",
            "gw_drive_delete",
        ),
        // Google Workspace (Forms)
        (
            r"(?i)\b(list|show|read|open|find|search)\b.*\b(google\s+forms?|forms?)\b",
            "gw_forms_list",
        ),
        (
            r"(?i)\b(create|new|make|build)\b.*\b(google\s+forms?|forms?)\b",
            "gw_forms_create",
        ),
        (r"(?i)\b(ping)\s+\w+", "ping_host"),
        (r"(?i)\b(download)\s+", "download_file"),
        (r"(?i)\b(my|public)\s*ip\b", "get_public_ip"),
        (r"(?i)\bdns\s+(lookup|resolve|query)\b", "dns_lookup"),
        (
            r"(?i)\bcheck.{0,20}url\b|\burl.{0,20}(status|reachable|accessible)\b",
            "check_url_status",
        ),
        // Internet connectivity check (must come AFTER specific internet patterns)
        (
            r"(?i)\b(connected|connection).{0,20}\b(internet|online|network)\b",
            "ping_host",
        ),
        (
            r"(?i)\b(internet|online)\b.{0,20}\b(connected|working|up|available|check)\b",
            "ping_host",
        ),
        (
            r"(?i)\bare\s+you\s+connected\b|\bam\s+i\s+online\b|\binternet\s+check\b",
            "ping_host",
        ),
        // Knowledge
        (r"(?i)\bremember\s+(that|this)\b", "remember_fact"),
        (
            r"(?i)\b(recall|what\s+did\s+I|do\s+you\s+remember)\b",
            "recall_fact",
        ),
        (
            r"(?i)\bsearch.{0,15}(my\s+)?(memory|knowledge)\b",
            "search_knowledge",
        ),
        (
            r"(?i)\blist.{0,20}(remember|snippets?|knowledge)\b",
            "list_remembered",
        ),
        // Notifications — keep general alert after dismiss_alert above
        (
            r"(?i)\b(notify|notification)\b|\bsend\s+(me\s+a\s+)?notification\b",
            "send_notification",
        ),
        (r"(?i)\b(remind|reminder)\s+me\b", "schedule_reminder"),
        (
            r"(?i)\b(email|compose|draft)\s*(an?\s+)?email\b",
            "compose_email",
        ),
        // Code execution
        (
            r"(?i)\b(run|execute)\s+(this\s+)?(bash|shell|command)\b",
            "execute_bash",
        ),
        (
            r"(?i)\b(run|execute)\s+(this\s+)?python\b",
            "execute_python",
        ),
        // Developer / git
        (r"(?i)\bgit\s+(status|stat)\b", "git_status"),
        (r"(?i)\bgit\s+(log|history|commits?)\b", "git_log"),
        (r"(?i)\bgit\s+diff\b", "git_diff"),
        (r"(?i)\bgit\s+(commit|save)\b", "git_commit"),
        (r"(?i)\bgit\s+(branch|branches)\b", "git_branch_list"),
        (r"(?i)\bgit\s+(stash)\b", "git_stash"),
        (r"(?i)\bgit\s+(push)\b", "git_push"),
        (r"(?i)\bgit\s+(checkout|switch)\b", "git_checkout"),
        (
            r"(?i)\banalyze.{0,20}(project|codebase|repo)\b",
            "analyze_project",
        ),
        // File ops extras
        (r"(?i)\bcount\s+(lines|loc)\b", "count_lines_of_code"),
        (
            r"(?i)\bproject\s+(structure|tree|layout)\b",
            "get_project_structure",
        ),
        (r"(?i)\b(find|show).{0,20}(todo|fixme)\b", "find_todos"),
        (
            r"(?i)\b(dir|folder)\s*(size|how\s+big)\b|\bhow\s+big.{0,20}(dir|folder|directory)\b",
            "calculate_dir_size",
        ),
        // Image generation — MUST come before vision "analyze image" rule to avoid shadowing.
        // Covers: "generate/create/make/draw/paint/design an image/picture/photo/art of ..."
        // Also handles: "draw me a robot", "make me an image"
        (
            r"(?i)\b(generate|create|make|draw|paint|design|render|produce)\s+(me\s+)?(a\s+|an\s+|one\s+)?\b(image|picture|photo|artwork|art|illustration|wallpaper|poster|banner|thumbnail)\b",
            "generate_image",
        ),
        // Handle "generate/draw/paint/create an image/photo/art OF ..."
        (
            r"(?i)\b(generate|create|make|draw|paint|design|render|produce)\b.{0,30}\b(image|picture|photo|artwork|art|illustration|wallpaper|poster|banner|thumbnail)\b",
            "generate_image",
        ),
        // Hinglish: "image banao", "photo bana", "tasveer banao"
        (
            r"(?i)\b(image|photo|tasveer|pic)\s*(banao?|bana|create|generate|draw)\b|\b(banao?|bana)\s*(ek\s+)?(image|photo|tasveer|pic)\b",
            "generate_image",
        ),
        // Vision extras
        (
            r"(?i)\b(ocr|extract\s+text|read\s+text).{0,20}(image|photo|picture|scan)\b",
            "ocr_image",
        ),
        (
            r"(?i)\b(analy|describe|identify|detect|summar).{0,30}(image|photo|picture|scan)\b|\bwhat.{0,20}\bin\s+(this\s+)?(image|photo|picture)\b",
            "analyze_image",
        ),
        (
            r"(?i)\bwhat.{0,20}\b(on\s+)?screen\b|\b(analy|describe).{0,20}(my\s+)?screen\b",
            "screenshot_analyze",
        ),
        // Article extraction (sidecar)
        (r"(?i)\bextract\s+(the\s+)?article\b", "web_extract_article"),
        // Embeddings (sidecar)
        (
            r"(?i)\b(generate|create|make|compute|get)\s+(text\s+)?embeddings?\b|\bembedding\s+for\b",
            "embeddings_generate",
        ),
        // Accessibility settings
        (
            r"(?i)\b(get|show|list|view|check)\b.{0,10}\baccessibility\b|^accessibility\s+settings\b",
            "get_accessibility_settings",
        ),
        // Languages list (must come BEFORE conversation 'what is/are' patterns via DIRECT_TOOL precedence)
        (
            r"(?i)\b(what|which)\s+languages?\s+(do\s+)?(you\s+)?(support|speak)\b|\blist\s+(supported\s+)?languages?\b",
            "list_languages",
        ),
        // Installed packages / applications listing
        (
            r"(?i)\blist\s+(all\s+)?(installed\s+)?(applications?|apps?|packages?|programs?)\b",
            "list_installed_packages",
        ),
        (
            r"(?i)\b(installed|all)\s+(applications?|apps?|packages?|programs?)\b",
            "list_installed_packages",
        ),
        // Fleet inventory / VM count queries
        (
            r"(?i)\b(how\s+many|count|number\s+of)\s+(?:my\s+)?(?:vms?|virtual\s+machines?|connected\s+(?:machines?|computers?|hosts?|laptops?)|remote\s+(?:machines?|hosts?|computers?))\b",
            "get_fleet_overview",
        ),
        (
            r"(?i)\b(list|show|which)\s+(?:all\s+)?(?:my\s+)?(?:vms?|virtual\s+machines?|connected\s+(?:machines?|computers?|hosts?|laptops?)|remote\s+(?:machines?|hosts?|computers?))\b",
            "get_fleet_overview",
        ),
        // Package — use correct tool name
        (r"(?i)\binstall\s+\w+\b", "install_package"),
        (r"(?i)\buninstall\s+\w+\b", "uninstall_package"),
        (
            r"(?i)\bremove\s+package\b|\bremove\s+\w+\s+package\b",
            "uninstall_package",
        ),
        // Hinglish patterns — fetch_webpage must come BEFORE generic Hinglish so URLs aren't lost
        // Web — fetch_webpage (placed after all Google Workspace patterns so gdocs/gsheets take priority)
        (
            r"(?i)\b(fetch|scrape|get|read|load)\b.{0,40}https?://",
            "fetch_webpage",
        ),
        (r"(?i)\bfetch\s+the\s+content\s+of\b", "fetch_webpage"),
        (
            r"(?i)\b(get|load|scrape|read)\s+the\s+(content|page|text|html)\b",
            "fetch_webpage",
        ),
        // ── Weather / Time / Currency / Calculator ──
        (
            r"(?i)\b(what('?s|\s+is)\s+the\s+)?weather\b",
            "get_weather",
        ),
        (
            r"(?i)\b(weather|mausam)\s+(today|tomorrow|now|forecast|kaisa|kya)\b",
            "get_weather",
        ),
        (
            r"(?i)\b(current|local)\s+time\b|\bwhat\s+time\s+is\s+it\b|\btime\s+(now|kya)\b",
            "get_current_time",
        ),
        (
            r"(?i)\btime\s+in\s+\w+\b",
            "get_current_time",
        ),
        (
            r"(?i)\b(convert|exchange)\s+\d+.*\b(to|into)\b.*\b(usd|eur|inr|gbp|jpy|currency)\b",
            "get_exchange_rate",
        ),
        (
            r"(?i)\bexchange\s+rate\b|\bcurrency\s+convert\b",
            "get_exchange_rate",
        ),
        (
            r"(?i)\b(calculate|compute|math|solve)\b.*[\d+\-*/^()]+",
            "calculate",
        ),
        (
            r"(?i)\b\d+\s*[\+\-\*/\^]\s*\d+",
            "calculate",
        ),
        // ── Power — extended patterns ──
        (
            r"(?i)\block\s+(my\s+)?(screen|computer|pc|laptop)\b",
            "lock_screen",
        ),
        (
            r"(?i)\bscreen\s+lock\s+(karo|karo|kar|lagao)\b",
            "lock_screen",
        ),
        (
            r"(?i)\b(hibernate|suspend\s+to\s+disk)\b",
            "hibernate",
        ),
        (
            r"(?i)\b(cancel|abort|stop)\s+(the\s+)?(shutdown|reboot|restart)\b",
            "execute_bash",
        ),
        // ── WiFi / Config — extended ──
        (
            r"(?i)\b(turn|switch)\s+(on|off|enable|disable)\s+(the\s+)?wifi\b",
            "toggle_wifi",
        ),
        (
            r"(?i)\bwifi\s+(on|off|enable|disable|toggle)\b",
            "toggle_wifi",
        ),
        (
            r"(?i)\bconnect\s+(to|with)\s+(the\s+)?(wifi|wi-fi|wireless|network)\b",
            "connect_wifi",
        ),
        (
            r#"(?i)\bconnect\s+(to|with)\s+['"]?\w+['"]?\s+(wifi|network|password)\b"#,
            "connect_wifi",
        ),
        // ── Environment variables ──
        (
            r"(?i)\bwhat\s+(is|are)\s+(the\s+)?(value\s+of\s+)?(the\s+)?\w*\s*(environment|env)\s*(var|variable)",
            "get_environment_variable",
        ),
        (
            r"(?i)\b(get|show|print|echo)\s+(the\s+)?(environment|env)\s*(var|variable)",
            "get_environment_variable",
        ),
        (
            r"(?i)\b(list|show)\s+(all\s+)?(environment|env)\s*(vars|variables)\b",
            "list_environment_variables",
        ),
        (
            r"(?i)\b(set|create|add|define)\s+(an?\s+)?(environment|env)\s*(var|variable)\b",
            "set_environment_variable",
        ),
        // ── File operations — extended ──
        (
            r"(?i)\b(create|make|mkdir)\s+(a\s+)?(folder|directory)\s+(at|in|on)\s+\S+",
            "create_directory",
        ),
        (
            r"(?i)\b(create|make|mkdir)\s+(a\s+)?(folder|directory)\b",
            "create_directory",
        ),
        (
            r#"(?i)\b(write|save|put)\s+['"].*['"]?\s+(to|in|into|at)\s+(/|~/)\S+"#,
            "write_file",
        ),
        (
            r"(?i)\b(write|save)\s+\S+\s+(to|into)\s+(the\s+)?(file|path)\b",
            "write_file",
        ),
        (
            r"(?i)\b(copy|cp)\s+(/|~/)\S+\s+(to|into)\s+(/|~/)\S+",
            "copy_file",
        ),
        (
            r"(?i)\bcopy\s+(the\s+)?(file|folder|directory)\b",
            "copy_file",
        ),
        (
            r"(?i)\b(rename|mv)\s+(/|~/)\S+\s+(to|as)\s+\S+",
            "rename_file",
        ),
        (
            r"(?i)\brename\s+(the\s+)?(file|folder|directory)\b",
            "rename_file",
        ),
        (
            r"(?i)\bmove\s+(/|~/)\S+\s+(to|into)\s+(/|~/)\S+",
            "move_file",
        ),
        (
            r"(?i)\bmove\s+(the\s+)?(file|folder|directory)\b",
            "move_file",
        ),
        (
            r"(?i)\b(get|show)\s+(info|information|details|metadata)\s+(about|for|of)\s+(/|~/)\S+",
            "get_file_info",
        ),
        (
            r"(?i)\b(info|details|metadata)\s+(about|for|of)\s+(/|~/)\S+",
            "get_file_info",
        ),
        (
            r"(?i)\b(delete|remove|rm)\s+(/|~/)\S+",
            "delete_file",
        ),
        (
            r"(?i)\b(delete|remove)\s+(the\s+)?(folder|directory)\s+(at|in)\s+\S+",
            "delete_directory",
        ),
        (
            r"(?i)\b(delete|remove)\s+(the\s+)?(folder|directory)\b",
            "delete_directory",
        ),
        (
            r"(?i)\bdiff\s+(/|~/)\S+\s+(and|with|vs)\s+(/|~/)\S+",
            "diff_files",
        ),
        (
            r"(?i)\b(show\s+)?(unified\s+)?diff\s+(between|of)\s+(/|~/)\S+",
            "diff_files_unified",
        ),
        (
            r"(?i)\bclean\s+(the\s+)?(temp|temporary|tmp)\s+files\b",
            "clean_temp_files",
        ),
        (
            r"(?i)\b(analy[sz]e|inspect|review)\s+(the\s+)?(code|source)\s+(in|at|of)\s+\S+",
            "analyze_code",
        ),
        (
            r"(?i)\b(find|search)\s+(all\s+)?\w+\s+files\s+(under|in|at)\s+\S+",
            "find_files_by_pattern",
        ),
        (
            r#"(?i)\b(search|grep|find)\s+(for\s+)?['"]?\w+['"]?\s+(in|across|through)\s+(all\s+)?(.+\s+)?files\b"#,
            "search_file_contents",
        ),
        // ── Process / Desktop — extended ──
        (
            r"(?i)\b(what|which)\s+(apps?|applications?|programs?|processes?)\s+(are\s+)?(running|active|open)\b",
            "list_running_apps",
        ),
        (
            r"(?i)\brunning\s+(apps?|applications?|processes?|programs?)\b",
            "list_running_apps",
        ),
        (
            r"(?i)\b(bring|focus|switch\s+to)\s+.{0,30}\b(to\s+front|forward|focus)\b",
            "focus_window",
        ),
        (
            r"(?i)\b(bring|focus)\s+\w+\s+(to\s+(the\s+)?front)\b",
            "focus_window",
        ),
        (
            r"(?i)\bset\s+(the\s+)?process\s+priority\b",
            "set_process_priority",
        ),
        (
            r"(?i)\bmaximize\s+\w+\b",
            "maximize_window",
        ),
        (
            r"(?i)\bminimize\s+\w+\b",
            "minimize_window",
        ),
        (
            r"(?i)\btile\s+.{1,40}\s+(and|with)\s+.{1,30}\b(side\s+by\s+side|together|split)\b",
            "tile_windows",
        ),
        (
            r"(?i)\btile\s+.{1,40}\s+(and|with)\s+\w+",
            "tile_windows",
        ),
        (
            r"(?i)\btile\s+(the\s+)?(windows?|apps?)\b",
            "tile_windows",
        ),
        (
            r"(?i)\bmove\s+.{1,40}\s+(window\s+)?(to\s+)?(x\s*=|position|coordinates?)\b",
            "move_window",
        ),
        (
            r"(?i)\bresize\s+.{1,40}\s+(window\s+)?(to\s+)?\d+\s*x\s*\d+",
            "resize_window",
        ),
        // ── Package management — extended ──
        (
            r"(?i)\b(is|check\s+if)\s+\w+\s+(installed|available)\b",
            "check_package_installed",
        ),
        (
            r"(?i)\b(check|verify)\s+if\s+(the\s+)?(package\s+)?\w+\s+is\s+installed\b",
            "check_package_installed",
        ),
        (
            r"(?i)\bsearch\s+(for\s+)?(the\s+)?package\s+\w+",
            "search_package",
        ),
        (
            r"(?i)\b(info|information|details)\s+(about|for)\s+(the\s+)?package\s+\w+",
            "get_package_info",
        ),
        (
            r"(?i)\b(check|are\s+there)\s+(any\s+)?updates?\s+(for|available\s+for)\s+\w+",
            "check_package_updates",
        ),
        (
            r"(?i)\bupdates?\s+(for|available\s+for)\s+\w+",
            "check_package_updates",
        ),
        // ── Scheduling ──
        (
            r"(?i)\b(create|add|set\s+up)\s+(a\s+)?(cron\s+job|scheduled\s+task|timer)\b",
            "create_scheduled_task",
        ),
        (
            r"(?i)\b(delete|remove|cancel)\s+(the\s+)?(cron\s+job|scheduled\s+task)\b",
            "delete_scheduled_task",
        ),
        // ── Knowledge / Memory — extended ──
        (
            r"(?i)\b(what\s+is|what('?s|\s+was)|tell\s+me)\s+my\s+\w+\b",
            "recall_fact",
        ),
        (
            r#"(?i)\b(get|show|retrieve)\s+(the\s+)?(snippet|code\s+snippet)\s+['"]?\w+"#,
            "get_snippet",
        ),
        (
            r"(?i)\b(ingest|import|load|index)\s+(the\s+)?(document|doc|pdf|file)\s+\S+\s+(into\s+)?(memory|knowledge|rag)\b",
            "ingest_document_rag",
        ),
        (
            r"(?i)\b(ingest|import|load|index)\s+\S+\s+(into\s+)?(knowledge\s+index|memory)\b",
            "ingest_document",
        ),
        (
            r"(?i)\b(ask|query|search)\s+(my\s+)?(knowledge\s+base|memory)\b",
            "rag_query",
        ),
        (
            r"(?i)\b(list|show)\s+(all\s+)?(documents?|items?)\s+(in\s+)?(my\s+)?(knowledge\s+base|memory)\b",
            "list_knowledge_base",
        ),
        (
            r"(?i)\b(delete|remove)\s+(knowledge|memory)\s+(item|entry|document)\b",
            "delete_knowledge_item",
        ),
        // ── Git — extended ──
        (
            r"(?i)\b(show|get|list)\s+(me\s+)?(the\s+)?(last|recent|latest)\s+\d+\s+(git\s+)?commits?\b",
            "git_log",
        ),
        (
            r"(?i)\bgit\s+log\b|\bcommit\s+history\b",
            "git_log",
        ),
        (
            r"(?i)\b(commit|save)\s+(all\s+)?(my\s+)?changes\b",
            "git_commit",
        ),
        (
            r"(?i)\b(create|make)\s+(and\s+)?(checkout|switch\s+to)\s+(a\s+)?(new\s+)?branch\b",
            "git_checkout",
        ),
        (
            r"(?i)\b(checkout|switch\s+to)\s+(the\s+)?branch\b",
            "git_checkout",
        ),
        (
            r"(?i)\b(stash|save)\s+(my\s+)?(the\s+)?changes\b",
            "git_stash",
        ),
        (
            r"(?i)\bpush\s+(to\s+)?(main|master|origin)\b",
            "execute_bash",
        ),
        // ── Automation ──
        (
            r"(?i)\b(watch|monitor)\s+(the\s+)?(directory|folder|dir)\s+\S+\s+(for\s+)?(changes?|modifications?)\b",
            "watch_directory",
        ),
        (
            r"(?i)\b(list|show)\s+(watched|monitored)\s+(directories|folders|dirs)\b",
            "list_watched_dirs",
        ),
        (
            r"(?i)\b(give\s+me\s+)?(a\s+)?smart\s+(suggestion|recommendation)\b",
            "smart_suggest",
        ),
        // ── i18n ──
        (
            r"(?i)\b(detect|identify|determine)\s+(the\s+)?language\s+(of|in)\b",
            "detect_language",
        ),
        // ── Hinglish calendar / schedule ──
        (
            r"(?i)\b(kal|aaj|aajka|aaj\s+ka)\s+(ka\s+)?(schedule|calendar|events?|meetings?)\b",
            "gw_calendar_search",
        ),
        (
            r"(?i)\b(schedule|calendar|events?|meetings?)\b.*\b(kal|aaj|aajka)\b",
            "gw_calendar_search",
        ),
        (
            r"(?i)\bmujhe\s+(kal|aaj)\s+ka\s+schedule\b",
            "gw_calendar_search",
        ),
        // ── Network — extended ──
        (
            r"(?i)\b(check|test)\s+if\s+\S+\s+(is\s+)?(reachable|accessible|up|alive)\b",
            "check_url_status",
        ),
        (
            r"(?i)\b(check|test)\s+url\s+(status|reachability)\b",
            "check_url_status",
        ),
        (
            r"(?i)\b(search|query)\s+(using|via|with)\s+searxng\b",
            "searxng_search",
        ),
        // ── News sources ──
        (
            r"(?i)\b(list|show)\s+(the\s+)?(news\s+)?(sources?|feeds?|monitors?)\b",
            "list_news_sources",
        ),
        (
            r"(?i)\bnews\s+(system\s+)?(status|health|state)\b",
            "news_status",
        ),
        // ── Document parsing ──
        (
            r"(?i)\b(parse|read|extract|process)\s+(the\s+)?(document|doc|pdf)\s+(at|in|from)\s+\S+",
            "parse_document",
        ),
        (
            r"(?i)\b(parse|read|process)\s+(the\s+)?(csv|spreadsheet)\s+(at|in|from)\s+\S+",
            "parse_csv",
        ),
        (
            r"(?i)\b(summarize|summarise)\s+(the\s+)?(document|doc|pdf)\s+(at|in|from)\s+\S+",
            "summarize_document",
        ),
        // ── Colab / Google Workspace — extended ──
        (
            r"(?i)\b(create|make|new)\s+(a\s+)?(google\s+)?colab\s+notebook\b",
            "gw_drive_create",
        ),
        (
            r"(?i)\b(open|start|connect)\s+(the\s+)?(colab\s+)?(browser\s+)?(connection|session)\b",
            "mcp_colab-mcp_open_colab_browser_connection",
        ),
        (
            r"(?i)\b(execute|run)\s+(this\s+)?(code|python|cell)\s+(in|on|at)\s+colab\b",
            "open_colab_browser_connection",
        ),
        (
            r"(?i)\bcolab\s+(chalao|start|open|run|execute)\b",
            "mcp_colab-mcp_open_colab_browser_connection",
        ),
        (
            r"(?i)\bcolab\s+mein\s+(notebook|code)\b",
            "gw_drive_create",
        ),
        // ── Direct tool invocation syntax ──
        (
            r"(?i)^!!tool:(\w+)",
            "DIRECT_TOOL_OVERRIDE",
        ),
        // ── Database ──
        (
            r"(?i)\b(describe|show|list)\s+(the\s+)?(database|db)\s+schema\b",
            "describe_database",
        ),
        // ── Screenshot + analyze ──
        (
            r"(?i)\b(take|capture)\s+(a\s+)?screenshot\s+(and|then)\s+(analyze|analyse|describe|ocr)\b",
            "screenshot_analyze",
        ),
        // ── Image on clipboard ──
        (
            r"(?i)\b(read|extract|get)\s+(the\s+)?text\s+(from|on)\s+(the\s+)?(image\s+on\s+)?clipboard\b",
            "get_clipboard",
        ),
        // Hinglish patterns
        (
            r"(?i)\bvolume\s+(band|zero|mute|off)\s+karo\b|\bband\s+karo\b.{0,15}volume",
            "set_volume",
        ),
        (
            r"(?i)\b(cpu|processor)\s+kitna\b|\bram\s+(kitna|kya)\b",
            "get_cpu_usage",
        ),
        (
            r"(?i)\binternet\s+(hai|check|nahi|connected)\b",
            "ping_host",
        ),
        (r"(?i)\bbattery\s+(kitna|kya|check)\b", "get_battery_status"),
    ];

    mappings
        .into_iter()
        .filter_map(|(pat, tool)| Regex::new(pat).ok().map(|r| (r, tool)))
        .collect()
});

// ─── Verb → category mapping for complex tasks ───
static VERB_TO_CATEGORY: Lazy<HashMap<&str, &str>> = Lazy::new(|| {
    let mut m = HashMap::new();
    for (verbs, cat) in [
        (
            &[
                "open", "launch", "start", "run", "focus", "close", "quit", "kill",
            ][..],
            "app_lifecycle",
        ),
        (
            &[
                "read", "write", "create", "delete", "move", "copy", "rename", "search", "find",
                "list",
            ][..],
            "file_ops",
        ),
        (&["install", "uninstall", "update"][..], "disk"),
        (
            &["volume", "brightness", "wifi", "bluetooth"][..],
            "system_config",
        ),
        (
            &["shutdown", "reboot", "sleep", "hibernate", "lock"][..],
            "power",
        ),
        (
            &["search", "google", "download", "fetch", "ping"][..],
            "internet",
        ),
        (&["remember", "recall", "forget", "save"][..], "knowledge"),
        (&["notify", "remind", "email"][..], "communication"),
        (&["screenshot", "clipboard", "type"][..], "interaction"),
        (&["schedule", "cron"][..], "scheduler"),
    ] {
        for verb in verbs {
            m.insert(*verb, cat);
        }
    }
    m
});

/// Intent router — classifies user text into an intent.
pub struct IntentRouter;

impl IntentRouter {
    /// Classify user input text.
    pub fn classify(text: &str) -> IntentResult {
        let trimmed = text.trim();

        // 0. Check for direct tool invocation syntax: !!tool:tool_name query=...
        if trimmed.starts_with("!!tool:") {
            let after_prefix = &trimmed[7..];
            let tool_name = after_prefix
                .split_whitespace()
                .next()
                .unwrap_or("")
                .split('=')
                .next()
                .unwrap_or("")
                .trim();
            if !tool_name.is_empty() {
                return IntentResult {
                    intent: Intent::DirectTool(tool_name.to_string()),
                    tool_hint: Some(tool_name.to_string()),
                    category: None,
                    confidence: 1.0,
                };
            }
        }

        // 1. Check direct tool patterns first (highest confidence)
        //    Confidence is computed from match quality: how many query tokens
        //    the regex actually consumed (avoiding blanket 0.85 for partial matches).
        for (re, tool) in DIRECT_TOOL_RE.iter() {
            if let Some(mat) = re.find(trimmed) {
                let match_len = mat.len() as f32;
                let query_len = trimmed.len() as f32;
                let coverage = if query_len > 0.0 { match_len / query_len } else { 0.0 };
                // Scale: full coverage → 0.95, half → 0.70, minimal → 0.55
                let dynamic_confidence = (0.55 + coverage * 0.40).min(0.95);
                return IntentResult {
                    intent: Intent::DirectTool(tool.to_string()),
                    tool_hint: Some(tool.to_string()),
                    category: None,
                    confidence: dynamic_confidence,
                };
            }
        }

        // 2. Check conversation patterns
        for re in CONVERSATION_RE.iter() {
            if re.is_match(trimmed) {
                return IntentResult {
                    intent: Intent::Conversation,
                    tool_hint: None,
                    category: None,
                    confidence: 0.75,
                };
            }
        }

        // 3. Check verb-based category mapping
        let first_word = trimmed
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_lowercase();
        if let Some(category) = VERB_TO_CATEGORY.get(first_word.as_str()) {
            return IntentResult {
                intent: Intent::ComplexTask,
                tool_hint: None,
                category: Some(category.to_string()),
                confidence: 0.6,
            };
        }

        // 4. Default: complex task (let LLM decide)
        IntentResult {
            intent: Intent::ComplexTask,
            tool_hint: None,
            category: None,
            confidence: 0.3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Intent, IntentRouter};

    #[test]
    fn routes_latest_news_prompts_to_search_news() {
        let result = IntentRouter::classify("Give me latest breaking news updates");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(result.tool_hint.as_deref(), Some("search_news"));
    }

    #[test]
    fn routes_region_news_prompts_to_search_news() {
        let result = IntentRouter::classify("Show trusted news from India about economy");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(result.tool_hint.as_deref(), Some("search_news"));
    }

    #[test]
    fn keeps_general_web_lookup_on_web_search() {
        let result = IntentRouter::classify("Search online for rust ownership examples");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(result.tool_hint.as_deref(), Some("web_search"));
    }

    #[test]
    fn routes_check_gmail_prompts_to_gmail_inbox_tool() {
        let result = IntentRouter::classify("check my gmail for unread emails");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(result.tool_hint.as_deref(), Some("gw_gmail_inbox"));
    }

    #[test]
    fn routes_search_gmail_prompts_to_gmail_search_tool() {
        let result = IntentRouter::classify("search gmail for from:boss subject:invoice");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(result.tool_hint.as_deref(), Some("gw_gmail_search"));
    }

    #[test]
    fn routes_fetch_latest_unread_gmails_to_gmail_inbox_tool() {
        let result = IntentRouter::classify("Fetch 3 latest unread gmails");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(result.tool_hint.as_deref(), Some("gw_gmail_inbox"));
    }

    #[test]
    fn routes_send_mail_prompts_to_gmail_send_tool() {
        let result = IntentRouter::classify("Send a Hye mail to \"zeeshanobaid335@gmail.com\"");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(result.tool_hint.as_deref(), Some("gw_gmail_send"));
    }

    #[test]
    fn routes_delete_gmail_prompts_to_gmail_delete_tool() {
        let result = IntentRouter::classify("Delete this email message_id 18af9f0a8bcdef12");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(result.tool_hint.as_deref(), Some("gw_gmail_delete"));
    }

    #[test]
    fn routes_schedule_meeting_to_calendar_create_tool() {
        let result = IntentRouter::classify("Schedule a Google Meet for tomorrow at 3pm");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(result.tool_hint.as_deref(), Some("gw_calendar_create"));
    }

    #[test]
    fn routes_calendar_cancel_prompts_to_calendar_delete_tool() {
        let result = IntentRouter::classify("Cancel my calendar event with event id abc123def456");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(result.tool_hint.as_deref(), Some("gw_calendar_delete"));
    }

    #[test]
    fn routes_create_doc_to_docs_create_tool() {
        let result = IntentRouter::classify("Create a new Google Doc called Weekly Plan");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(result.tool_hint.as_deref(), Some("gw_docs_create"));
    }

    #[test]
    fn routes_forms_listing_to_curated_forms_tool() {
        let result = IntentRouter::classify("List my google forms");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(result.tool_hint.as_deref(), Some("gw_forms_list"));
    }

    #[test]
    fn routes_actionable_question_to_calendar_today_tool() {
        let result = IntentRouter::classify("Do I have meetings today?");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(result.tool_hint.as_deref(), Some("gw_calendar_today"));
    }

    #[test]
    fn routes_todays_calendar_phrase_to_calendar_today_tool() {
        let result = IntentRouter::classify("today's calendar");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(result.tool_hint.as_deref(), Some("gw_calendar_today"));
    }

    #[test]
    fn routes_drive_listing_prompts_to_drive_list_tool() {
        let result = IntentRouter::classify("List files in my Google drive");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(result.tool_hint.as_deref(), Some("gw_drive_list"));
    }

    #[test]
    fn routes_drive_read_prompts_to_drive_read_tool() {
        let result = IntentRouter::classify("Read this file from Google Drive");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(result.tool_hint.as_deref(), Some("gw_drive_read"));
    }

    #[test]
    fn routes_docs_delete_prompts_to_drive_delete_tool() {
        let result = IntentRouter::classify("Delete this Google Doc");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(result.tool_hint.as_deref(), Some("gw_drive_delete"));
    }

    #[test]
    fn routes_sheets_delete_prompts_to_drive_delete_tool() {
        let result = IntentRouter::classify("Remove this spreadsheet from my drive");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(result.tool_hint.as_deref(), Some("gw_drive_delete"));
    }

    #[test]
    fn routes_calendar_update_prompts_to_calendar_search_tool() {
        let result = IntentRouter::classify("Get latest updates about Google calendar");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(result.tool_hint.as_deref(), Some("gw_calendar_search"));
    }

    #[test]
    fn routes_folder_lookup_prompts_to_search_files() {
        let result = IntentRouter::classify("search for folder name zrok");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(result.tool_hint.as_deref(), Some("search_files"));
    }

    #[test]
    fn routes_image_analysis_prompts_to_analyze_image() {
        let result = IntentRouter::classify("Analyze this image");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(result.tool_hint.as_deref(), Some("analyze_image"));
    }

    #[test]
    fn routes_screen_analysis_prompts_to_screenshot_analyze() {
        let result = IntentRouter::classify("What is on my screen right now?");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(result.tool_hint.as_deref(), Some("screenshot_analyze"));
    }

    #[test]
    fn routes_vm_count_queries_to_fleet_overview() {
        let result = IntentRouter::classify("How many VMs i have?");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(result.tool_hint.as_deref(), Some("get_fleet_overview"));
    }

    #[test]
    fn routes_connected_machine_listing_to_fleet_overview() {
        let result = IntentRouter::classify("List my connected machines");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(result.tool_hint.as_deref(), Some("get_fleet_overview"));
    }

    #[test]
    fn routes_generic_vm_health_prompt_to_check_device_health() {
        let result = IntentRouter::classify("is my VM up?");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(result.tool_hint.as_deref(), Some("check_device_health"));
    }

    #[test]
    fn routes_server_status_prompt_to_check_device_health() {
        let result = IntentRouter::classify("check status of the server");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(result.tool_hint.as_deref(), Some("check_device_health"));
    }

    #[test]
    fn routes_list_installed_apps_to_list_installed_packages() {
        let result = IntentRouter::classify("List the Apps installed in my System");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(
            result.tool_hint.as_deref(),
            Some("list_installed_packages")
        );
    }

    #[test]
    fn routes_show_installed_packages_to_list_installed_packages() {
        let result = IntentRouter::classify("show me all installed packages");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(
            result.tool_hint.as_deref(),
            Some("list_installed_packages")
        );
    }

    #[test]
    fn routes_what_programs_are_installed_to_list_installed_packages() {
        let result = IntentRouter::classify("what programs are installed on my system");
        assert!(matches!(result.intent, Intent::DirectTool(_)));
        assert_eq!(
            result.tool_hint.as_deref(),
            Some("list_installed_packages")
        );
    }
}
