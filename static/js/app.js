const PROFILE_STORAGE_KEY = "strawberry-social-profiles-v1";
const ACTIVE_PROFILE_KEY = "strawberry-social-active-profile-v1";
const THEME_KEY = "strawberry-social-theme-v1"; // Used only once to migrate the previous browser preference.

const state = {
    config: {}, characters: [], sessions: [], profiles: [], activeProfileId: null,
    currentSessionId: null, currentCharacter: null, currentCard: null, editingProfileId: null,
    runtimeInfoLoaded: false, runtimeInfoLoading: false,
    serverStatus: null, installedModels: [], selectedModelFiles: null, downloadId: null,
    modelBrowserLoaded: false, modelBrowserLoading: false, lastDownloadStatus: "idle", dismissedDownloadId: null
};

const $ = (selector, root = document) => root.querySelector(selector);
const $$ = (selector, root = document) => [...root.querySelectorAll(selector)];

document.addEventListener("DOMContentLoaded", initializeApp);

async function initializeApp() {
    bindNavigation();
    bindChat();
    bindUploads();
    bindProfiles();
    bindSettings();
    bindModelControls();
    bindApplicationControls();
    bindCharacterDrawer();
    await loadProfiles();
    applyTheme(state.theme || "dark");
    await Promise.all([loadConfig(), loadCharacters(), loadSessions()]);
    renderProfiles();
    await Promise.all([loadInstalledModels(), refreshServerStatus(), refreshDownloadStatus()]);
    window.setInterval(() => { refreshServerStatus(); refreshDownloadStatus(); }, 1200);
}

function bindNavigation() {
    $$(".nav-item").forEach(button => button.addEventListener("click", () => navigate(button.dataset.view)));
    window.addEventListener("hashchange", () => navigate(location.hash.slice(1) || "chat", false));
    const initial = location.hash.slice(1);
    if (["chat", "profiles", "models", "settings"].includes(initial)) navigate(initial, false);
}

function bindApplicationControls() {
    const button = $("#shutdown-app-btn");
    const status = $("#application-status-text");
    button.addEventListener("click", async () => {
        const confirmed = window.confirm(
            "Close Strawberry Social now? This stops the web server and any managed llama.cpp process, and disconnects open chats."
        );
        if (!confirmed) return;

        button.disabled = true;
        setApplicationStatus("closing", "Closing…");
        status.textContent = "Closing…";
        try {
            const response = await fetch("/api/application/shutdown", { method: "POST" });
            if (!response.ok) throw new Error(await response.text());
            setApplicationStatus("closing", "Closing app");
        } catch (error) {
            setApplicationStatus("running", "App running");
            button.disabled = false;
            window.alert(`Could not close Strawberry Social: ${error.message}`);
        }
    });
    $("#dismiss-model-download-btn").addEventListener("click", () => {
        state.dismissedDownloadId = state.downloadId;
        $("#model-download-progress").hidden = true;
    });
}

function setApplicationStatus(stateName, label) {
    const status = $(".application-status");
    status.dataset.state = stateName;
    status.setAttribute("aria-label", `Strawberry Social is ${stateName}`);
    $("#application-status-text").textContent = label;
}

function navigate(view, updateHash = true) {
    if (!["chat", "profiles", "models", "settings"].includes(view)) view = "chat";
    if (view !== "settings") state.runtimeInfoLoaded = false;
    $$(".app-view").forEach(section => {
        const active = section.id === `view-${view}`;
        section.hidden = !active;
        section.classList.toggle("active", active);
    });
    $$(".nav-item").forEach(button => button.classList.toggle("active", button.dataset.view === view));
    if (updateHash && location.hash !== `#${view}`) history.replaceState(null, "", `#${view}`);
    if (view === "settings") {
        if (!state.runtimeInfoLoaded) loadRuntimeInfo();
    }
    if (view === "models") {
        if (!state.modelBrowserLoaded) loadHubModels();
        loadInstalledModels();
    }
}

async function loadConfig() {
    try {
        const response = await fetch("/api/config");
        if (!response.ok) throw new Error(await response.text());
        state.config = await response.json();
        $("#api-base-url").value = state.config.api_base_url || "";
        $("#context-window").value = state.config.context_window ?? 4096;
        $("#temperature").value = Number(state.config.temperature ?? 0.7).toFixed(2);
        $("#top-p").value = Number(state.config.top_p ?? 0.9).toFixed(2);
    } catch (error) { console.error("Failed to load settings:", error); }
}

async function loadCharacters() {
    try {
        const response = await fetch("/api/characters");
        if (!response.ok) throw new Error(await response.text());
        state.characters = await response.json();
        renderCharacters();
    } catch (error) {
        console.error("Failed to load characters:", error);
        $("#character-list").innerHTML = '<div class="loading-text">Could not load characters.</div>';
    }
}

function renderCharacters() {
    const container = $("#character-list");
    if (!state.characters.length) {
        container.innerHTML = '<div class="loading-text">No character cards yet.</div>';
        return;
    }
    container.replaceChildren(...state.characters.map(character => {
        const button = document.createElement("button");
        button.className = `roster-item ${state.currentCharacter?.name === character.name ? "active" : ""}`;
        button.type = "button";
        if (character.avatar_url) {
            const avatar = document.createElement("img"); avatar.src = character.avatar_url;
            avatar.alt = ""; avatar.className = "avatar-thumb"; button.append(avatar);
        } else {
            const avatar = document.createElement("span"); avatar.className = "avatar-thumb fallback";
            avatar.textContent = (character.name || "?").charAt(0); button.append(avatar);
        }
        const info = document.createElement("span"); info.className = "roster-info";
        const name = document.createElement("strong"); name.textContent = character.name;
        const creator = document.createElement("small"); creator.textContent = character.creator || "Unknown creator";
        info.append(name, creator); button.append(info);
        button.addEventListener("click", () => createSession(character));
        return button;
    }));
}

async function createSession(character) {
    try {
        const response = await fetch("/api/chat/sessions", {
            method: "POST", headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ character_name: character.name, title: `Chat with ${character.name}` })
        });
        if (!response.ok) throw new Error(await response.text());
        const session = await response.json();
        state.currentCharacter = character;
        state.currentCard = character;
        state.currentSessionId = session.id;
        renderCharacters();
        renderChatHeader(`Chat with ${character.name}`, true);
        setChatEnabled(true);
        await Promise.all([loadSessions(), loadMessages(session.id)]);
        navigate("chat");
        $("#message-input").focus();
    } catch (error) { console.error("Failed to create chat:", error); showChatError("Could not start this chat."); }
}

async function loadSessions() {
    try {
        const response = await fetch("/api/chat/sessions");
        if (!response.ok) throw new Error(await response.text());
        state.sessions = await response.json();
        renderSessions();
    } catch (error) { console.error("Failed to load chats:", error); }
}

function renderSessions() {
    const container = $("#session-list");
    if (!state.sessions.length) {
        container.innerHTML = '<div class="loading-text">Your chats will appear here.</div>';
        return;
    }
    container.replaceChildren(...state.sessions.map(session => {
        const row = document.createElement("div");
        row.className = `session-item ${session.id === state.currentSessionId ? "active" : ""}`;
        const open = document.createElement("button"); open.type = "button"; open.className = "session-open";
        const title = document.createElement("strong"); title.textContent = session.title;
        const character = document.createElement("small"); character.textContent = session.character_name;
        open.append(title, character); open.addEventListener("click", () => switchSession(session));
        const remove = document.createElement("button"); remove.type = "button"; remove.className = "delete-session-btn";
        remove.title = "Delete chat"; remove.setAttribute("aria-label", `Delete ${session.title}`); remove.textContent = "×";
        remove.addEventListener("click", () => deleteSession(session.id));
        row.append(open, remove); return row;
    }));
}

async function switchSession(session) {
    state.currentSessionId = session.id;
    const card = parseCard(session.character_card_json);
    state.currentCard = card;
    state.currentCharacter = card ? { name: session.character_name, avatar_url: card.avatar_url || null } : { name: session.character_name };
    renderChatHeader(`Chat with ${session.character_name}`, !!card);
    renderSessions(); renderCharacters(); setChatEnabled(true);
    navigate("chat");
    await loadMessages(session.id);
}

function parseCard(json) {
    if (!json) return null;
    try { const card = JSON.parse(json); card.name ||= "Character"; return card; }
    catch (error) { console.error("Invalid character snapshot:", error); return null; }
}

function renderChatHeader(title, hasCard) {
    $("#current-chat-title").textContent = title;
    $("#profile-btn").hidden = !hasCard;
}

function setChatEnabled(enabled) {
    $("#message-input").disabled = !enabled;
    $("#send-btn").disabled = !enabled;
}

async function loadMessages(sessionId) {
    try {
        const response = await fetch(`/api/chat/sessions/${sessionId}/messages`);
        if (!response.ok) throw new Error(await response.text());
        const messages = await response.json();
        const container = $("#chat-messages"); container.replaceChildren();
        if (!messages.length) { container.append(emptyState("Start the conversation", "Say hello to your character.")); return; }
        messages.forEach(message => appendMessageToUI(message.role, message.content));
    } catch (error) { console.error("Failed to load messages:", error); showChatError("Could not load this conversation."); }
}

function bindChat() { $("#chat-form").addEventListener("submit", handleSendMessage); }

async function handleSendMessage(event) {
    event.preventDefault();
    if (!state.currentSessionId) return;
    const input = $("#message-input"); const content = input.value.trim();
    if (!content) return;
    input.value = ""; setChatLocked(true); appendMessageToUI("user", content);
    try {
        const response = await fetch(`/api/chat/sessions/${state.currentSessionId}/messages`, {
            method: "POST", headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ content, user_profile: getActiveProfile() })
        });
        if (!response.ok) throw new Error(await response.text());
        const data = await response.json(); appendMessageToUI("assistant", data.assistant_message.content);
        await loadSessions();
    } catch (error) {
        console.error("Failed to send message:", error);
        appendMessageToUI("assistant", "⚠️ Could not reach the configured inference server.");
    } finally { setChatLocked(false); input.focus(); }
}

function setChatLocked(locked) {
    $("#message-input").disabled = locked || !state.currentSessionId;
    $("#send-btn").disabled = locked || !state.currentSessionId;
    $$(".app-sidebar button").forEach(button => button.disabled = locked);
}

function appendMessageToUI(role, text) {
    const container = $("#chat-messages"); const banner = $(".welcome-banner", container); if (banner) banner.remove();
    const wrapper = document.createElement("div"); wrapper.className = `message-wrapper ${role}`;
    const avatar = document.createElement("div"); avatar.className = "msg-avatar";
    if (role === "assistant" && state.currentCharacter?.avatar_url) {
        const image = document.createElement("img"); image.src = state.currentCharacter.avatar_url; image.alt = ""; avatar.append(image);
    } else {
        const fallback = document.createElement("div"); fallback.className = `fallback ${role === "user" ? "user-initial" : ""}`;
        fallback.textContent = role === "user" ? ((getActiveProfile()?.name || "Y").charAt(0).toUpperCase()) : (state.currentCharacter?.name || "?").charAt(0);
        avatar.append(fallback);
    }
    const bubble = document.createElement("div"); bubble.className = `message ${role}`;
    bubble.innerHTML = role === "assistant" ? formatMessage(text) : escapeHtml(text).replace(/\n/g, "<br>");
    if (role === "user") wrapper.append(bubble, avatar); else wrapper.append(avatar, bubble);
    container.append(wrapper); container.scrollTop = container.scrollHeight;
}

function formatMessage(text) {
    return escapeHtml(text).replace(/\n/g, "<br>").replace(/\*(.*?)\*/g, '<em class="rp-action">$1</em>');
}
function escapeHtml(value) { return String(value).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/'/g, "&#039;"); }
function emptyState(title, description) {
    const element = document.createElement("div"); element.className = "welcome-banner";
    const heading = document.createElement("h2"); heading.textContent = title;
    const body = document.createElement("p"); body.textContent = description;
    element.append(heading, body); return element;
}
function showChatError(message) { const container = $("#chat-messages"); container.replaceChildren(emptyState("Something went wrong", message)); }

async function deleteSession(id) {
    if (!confirm("Delete this chat and its message history?")) return;
    try {
        const response = await fetch(`/api/chat/sessions/${id}`, { method: "DELETE" });
        if (!response.ok) throw new Error(await response.text());
        if (state.currentSessionId === id) {
            state.currentSessionId = null; state.currentCharacter = null; state.currentCard = null;
            renderChatHeader("Choose a character to begin", false); setChatEnabled(false);
            $("#chat-messages").replaceChildren(emptyState("A little conversation?", "Choose a character from the sidebar to start a chat."));
        }
        await loadSessions();
    } catch (error) { console.error("Failed to delete chat:", error); }
}

function bindUploads() {
    const input = $("#file-input"); $("#upload-btn").addEventListener("click", () => input.click());
    input.addEventListener("change", async () => {
        const file = input.files[0]; if (!file) return;
        const data = new FormData(); data.append("file", file);
        try {
            const response = await fetch("/api/characters", { method: "POST", body: data });
            if (!response.ok) throw new Error(await response.text());
            await loadCharacters();
        } catch (error) { console.error("Upload failed:", error); alert(`Upload failed: ${error.message}`); }
        finally { input.value = ""; }
    });
}

async function loadProfiles() {
    const response = await fetch("/api/profiles");
    if (!response.ok) throw new Error(`Could not load profiles: ${await response.text()}`);
    const stored = await response.json();
    state.profiles = stored.profiles || [];
    state.activeProfileId = stored.active_profile_id || null;
    state.theme = stored.theme || "dark";

    // Import profiles saved by the previous browser-local version once, then remove that copy.
    let legacyProfiles = [];
    try { legacyProfiles = JSON.parse(localStorage.getItem(PROFILE_STORAGE_KEY) || "[]"); } catch { /* ignore malformed legacy data */ }
    if (Array.isArray(legacyProfiles) && legacyProfiles.length) {
        for (const profile of legacyProfiles) {
            const saveResponse = await fetch("/api/profiles", {
                method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(profile)
            });
            if (!saveResponse.ok) throw new Error(`Could not import profile: ${await saveResponse.text()}`);
        }
        if (!state.activeProfileId) {
            const legacyActiveId = localStorage.getItem(ACTIVE_PROFILE_KEY);
            const activeId = legacyProfiles.some(profile => profile.id === legacyActiveId) ? legacyActiveId : legacyProfiles[0].id;
            await setActiveProfile(activeId);
        }
        localStorage.removeItem(PROFILE_STORAGE_KEY);
        localStorage.removeItem(ACTIVE_PROFILE_KEY);
        const refresh = await fetch("/api/profiles");
        if (refresh.ok) {
            const updated = await refresh.json(); state.profiles = updated.profiles || [];
            state.activeProfileId = updated.active_profile_id || null; state.theme = updated.theme || state.theme;
        }
    }
    const legacyTheme = localStorage.getItem(THEME_KEY);
    if (legacyTheme === "dark" || legacyTheme === "light") {
        const themeResponse = await fetch("/api/preferences/theme", {
            method: "PUT", headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ theme: legacyTheme })
        });
        if (!themeResponse.ok) throw new Error(`Could not import theme preference: ${await themeResponse.text()}`);
        state.theme = legacyTheme;
        localStorage.removeItem(THEME_KEY);
    }
    localStorage.removeItem(PROFILE_STORAGE_KEY);
    localStorage.removeItem(ACTIVE_PROFILE_KEY);
}
async function setActiveProfile(id) {
    const response = await fetch("/api/profiles/active", {
        method: "PUT", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ active_profile_id: id || null })
    });
    if (!response.ok) throw new Error(`Could not set active profile: ${await response.text()}`);
    state.activeProfileId = id || null;
}
function getActiveProfile() { return state.profiles.find(profile => profile.id === state.activeProfileId) || null; }

function renderProfiles() {
    const list = $("#profile-list");
    if (!state.profiles.length) list.replaceChildren(emptyState("No profiles yet", "Create a profile to tell characters who you are."));
    else list.replaceChildren(...state.profiles.map(profile => {
        const card = document.createElement("button"); card.type = "button";
        card.className = `profile-card ${profile.id === state.activeProfileId ? "selected" : ""}`;
        const initial = document.createElement("span"); initial.className = "profile-initial"; initial.textContent = profile.name.charAt(0).toUpperCase();
        const info = document.createElement("span"); info.className = "profile-card-info";
        const name = document.createElement("strong"); name.textContent = profile.name;
        const description = document.createElement("small"); description.textContent = profile.description || "No description yet";
        info.append(name, description); card.append(initial, info);
        if (profile.id === state.activeProfileId) { const active = document.createElement("span"); active.className = "active-badge"; active.textContent = "Active"; card.append(active); }
        card.addEventListener("click", () => editProfile(profile.id)); return card;
    }));
    const select = $("#chat-profile-select"); select.replaceChildren();
    if (!state.profiles.length) {
        const option = document.createElement("option"); option.value = ""; option.textContent = "No profile"; select.append(option);
    } else state.profiles.forEach(profile => {
        const option = document.createElement("option"); option.value = profile.id; option.textContent = profile.name;
        option.selected = profile.id === state.activeProfileId; select.append(option);
    });
    select.disabled = !state.profiles.length;
    const activeProfile = getActiveProfile();
    $("#sidebar-profile").innerHTML = activeProfile ? `<span class="profile-initial">${escapeHtml(activeProfile.name.charAt(0).toUpperCase())}</span><span><strong>${escapeHtml(activeProfile.name)}</strong><small>Active profile</small></span><button class="icon-button" id="edit-active-profile" title="Edit profile">···</button>` : '<span class="profile-initial muted">?</span><span><strong>No profile</strong><small>Set up your chat persona</small></span>';
    $("#edit-active-profile")?.addEventListener("click", () => { navigate("profiles"); if (activeProfile) editProfile(activeProfile.id); });
}

function bindProfiles() {
    $("#new-profile-btn").addEventListener("click", () => resetProfileForm());
    $("#cancel-profile-btn").addEventListener("click", () => resetProfileForm());
    $("#delete-profile-btn").addEventListener("click", deleteProfile);
    $("#profile-form").addEventListener("submit", saveProfile);
    $("#chat-profile-select").addEventListener("change", event => {
        setActiveProfile(event.target.value || null).then(renderProfiles).catch(error => alert(error.message));
    });
}

function editProfile(id) {
    const profile = state.profiles.find(item => item.id === id); if (!profile) return;
    state.editingProfileId = id; $("#profile-id").value = id; $("#profile-name").value = profile.name;
    $("#profile-description").value = profile.description || ""; $("#profile-instructions").value = profile.instructions || "";
    $("#profile-editor-title").textContent = "Edit profile"; $("#delete-profile-btn").hidden = false;
    setActiveProfile(id).then(renderProfiles).catch(error => alert(error.message));
}
function resetProfileForm() {
    state.editingProfileId = null; $("#profile-form").reset(); $("#profile-id").value = "";
    $("#profile-editor-title").textContent = "New profile"; $("#delete-profile-btn").hidden = true;
}
async function saveProfile(event) {
    event.preventDefault();
    const profile = {
        id: state.editingProfileId || crypto.randomUUID(), name: $("#profile-name").value.trim(),
        description: $("#profile-description").value.trim(), instructions: $("#profile-instructions").value.trim()
    };
    if (!profile.name) return;
    try {
        const response = await fetch("/api/profiles", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(profile) });
        if (!response.ok) throw new Error(await response.text());
        await setActiveProfile(profile.id);
        await refreshProfiles(); renderProfiles(); resetProfileForm();
    } catch (error) { alert(`Could not save profile: ${error.message}`); }
}
async function refreshProfiles() {
    const response = await fetch("/api/profiles"); if (!response.ok) throw new Error(await response.text());
    const data = await response.json(); state.profiles = data.profiles || []; state.activeProfileId = data.active_profile_id || null;
}
async function deleteProfile() {
    const id = state.editingProfileId; if (!id) return;
    try {
        const response = await fetch(`/api/profiles/${encodeURIComponent(id)}`, { method: "DELETE" });
        if (!response.ok) throw new Error(await response.text());
        await refreshProfiles();
        if (!state.activeProfileId && state.profiles[0]) await setActiveProfile(state.profiles[0].id);
        await refreshProfiles(); renderProfiles(); resetProfileForm();
    } catch (error) { alert(`Could not delete profile: ${error.message}`); }
}

function bindSettings() {
    $("#theme-select").value = state.theme || "dark";
    $("#theme-select").addEventListener("change", async event => {
        const theme = event.target.value;
        try {
            const response = await fetch("/api/preferences/theme", { method: "PUT", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ theme }) });
            if (!response.ok) throw new Error(await response.text());
            state.theme = theme; applyTheme(theme);
        } catch (error) { alert(`Could not save theme: ${error.message}`); event.target.value = state.theme; }
    });
    $("#retry-runtime-info").addEventListener("click", () => loadRuntimeInfo(true));
    $("#download-runtime-btn").addEventListener("click", () => downloadRuntime(false));
    $("#auto-download-runtime").addEventListener("change", async event => {
        const enabled = event.target.checked;
        try {
            const response = await fetch("/api/preferences/auto-download", {
                method: "PUT", headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ enabled })
            });
            if (!response.ok) throw new Error(await response.text());
        } catch (error) {
            event.target.checked = !enabled;
            alert(`Could not save the download preference: ${error.message}`);
        }
    });
    $("#generation-settings-form").addEventListener("submit", saveGenerationSettings);
}

async function saveGenerationSettings(event) {
    event.preventDefault();
    const payload = {
        context_window: Number($("#context-window").value),
        temperature: Number($("#temperature").value),
        top_p: Number($("#top-p").value)
    };
    const status = $("#generation-settings-status");
    status.textContent = "Saving…";
    try {
        const response = await fetch("/api/config/generation", {
            method: "PUT", headers: { "Content-Type": "application/json" }, body: JSON.stringify(payload)
        });
        if (!response.ok) throw new Error(await response.text());
        Object.assign(state.config, payload);
        status.textContent = "Response settings saved.";
    } catch (error) {
        status.textContent = `Could not save response settings: ${error.message}`;
    }
}

function bindModelControls() {
    $("#model-search-form").addEventListener("submit", event => {
        event.preventDefault();
        loadHubModels(true);
    });
    $("#close-model-files-btn").addEventListener("click", () => { $("#model-download-panel").hidden = true; });
    $("#include-mmproj").addEventListener("change", updateMmprojSelection);
    $("#start-model-download-btn").addEventListener("click", startModelDownload);
    $("#pause-model-download-btn").addEventListener("click", () => controlModelDownload("pause"));
    $("#resume-model-download-btn").addEventListener("click", () => controlModelDownload("resume"));
    $("#cancel-model-download-btn").addEventListener("click", () => controlModelDownload("cancel"));
}

async function loadHubModels(force = false) {
    if (state.modelBrowserLoading || (state.modelBrowserLoaded && !force)) return;
    state.modelBrowserLoading = true;
    const status = $("#model-browser-status");
    const results = $("#model-search-results");
    const min = $("#model-min-parameters").value;
    const max = $("#model-max-parameters").value;
    const search = $("#model-search-text").value.trim();
    status.textContent = search
        ? `Searching Hugging Face for “${search}”…`
        : "Loading trending and popular Hugging Face models…";
    results.replaceChildren();
    try {
        const query = new URLSearchParams();
        if (min !== "any") query.set("min", min);
        if (max !== "any") query.set("max", max);
        if (search) query.set("search", search);
        const response = await fetch(`/api/models/search?${query}`);
        if (!response.ok) throw new Error(await response.text());
        const models = await response.json();
        state.modelBrowserLoaded = true;
        renderHubModels(models);
        status.textContent = search
            ? `${models.length} results for “${search}” · trending first, then additional results from downloads.`
            : `${models.length} models · trending first, then additional results from downloads.`;
    } catch (error) {
        status.textContent = `Could not load Hugging Face models: ${error.message}`;
        const retry = document.createElement("button");
        retry.type = "button"; retry.className = "secondary-button"; retry.textContent = "Retry model search";
        retry.addEventListener("click", () => loadHubModels(true));
        results.append(retry);
    } finally { state.modelBrowserLoading = false; }
}

function renderHubModels(models) {
    const container = $("#model-search-results");
    container.replaceChildren();
    if (!models.length) {
        container.append(emptyState("No matching GGUF models", "Try another model or provider name, or widen the parameter range."));
        return;
    }
    models.forEach(model => {
        const row = document.createElement("article"); row.className = "model-result";
        const main = document.createElement("div"); main.className = "model-result-main";
        const name = document.createElement("h3"); name.textContent = model.repo_id;
        const meta = document.createElement("div"); meta.className = "model-result-meta";
        meta.append(modelMetaItem(formatParameterCount(model.parameter_count, model.parameter_count_estimated)));
        meta.append(modelMetaItem(`Updated ${formatModelDate(model.last_updated)}`));
        if (Number.isFinite(model.downloads)) meta.append(modelMetaItem(`${formatCount(model.downloads)} downloads`));
        main.append(name, meta);
        const link = document.createElement("a"); link.href = model.url; link.target = "_blank"; link.rel = "noopener noreferrer"; link.textContent = "View on Hugging Face ↗";
        const actions = document.createElement("div"); actions.className = "model-result-actions";
        if (model.installed) {
            const badge = document.createElement("span"); badge.className = "model-installed-badge"; badge.textContent = "Downloaded"; actions.append(badge);
        }
        const choose = document.createElement("button"); choose.type = "button"; choose.className = model.installed ? "secondary-button" : "primary-button";
        choose.textContent = model.installed ? "Manage files" : "Choose files";
        choose.addEventListener("click", () => loadModelFiles(model)); actions.append(choose);
        row.append(main, link, actions); container.append(row);
    });
}

function modelMetaItem(text) {
    const item = document.createElement("span");
    item.textContent = text;
    return item;
}

async function loadModelFiles(model) {
    const panel = $("#model-download-panel");
    const description = $("#selected-repo-description");
    panel.hidden = false;
    $("#selected-repo-title").textContent = model.repo_id;
    description.textContent = "Loading available GGUF files and sizes…";
    panel.scrollIntoView({ behavior: "smooth", block: "start" });
    $("#selected-repo-title").focus({ preventScroll: true });
    try {
        const response = await fetch(`/api/models/files?repo_id=${encodeURIComponent(model.repo_id)}`);
        if (!response.ok) throw new Error(await response.text());
        const data = await response.json();
        state.selectedModelFiles = { ...data, model };
        const sortedModels = [...data.model_files].sort((a, b) => quantizationRank(a.quantization) - quantizationRank(b.quantization));
        populateModelFileSelect($("#selected-model-file"), sortedModels);
        const hasMmproj = data.mmproj_files.length > 0;
        $("#mmproj-toggle-row").hidden = !hasMmproj;
        $("#include-mmproj").checked = false;
        $("#selected-mmproj-file").replaceChildren();
        populateModelFileSelect($("#selected-mmproj-file"), data.mmproj_files);
        updateMmprojSelection();
        description.textContent = `${data.repo_id} · ${formatModelDate(model.last_updated)} · select the model quantization and optionally include a companion projector.`;
    } catch (error) {
        description.textContent = `Could not load repository files: ${error.message}`;
    }
}

function populateModelFileSelect(select, files) {
    select.replaceChildren();
    files.forEach(file => {
        const option = document.createElement("option"); option.value = file.path;
        option.textContent = `${file.quantization} · ${file.path} · ${formatBytes(file.size)}`;
        select.append(option);
    });
}

function updateMmprojSelection() {
    const show = $("#include-mmproj").checked && !!state.selectedModelFiles?.mmproj_files.length;
    $("#mmproj-label").hidden = !show;
    $("#selected-mmproj-file").hidden = !show;
}

async function startModelDownload() {
    const files = state.selectedModelFiles;
    if (!files) return;
    const modelFile = $("#selected-model-file").value;
    const mmprojFile = $("#include-mmproj").checked ? $("#selected-mmproj-file").value : null;
    if (!modelFile) return;
    try {
        const response = await fetch("/api/models/download", {
            method: "POST", headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ repo_id: files.repo_id, revision: files.revision, model_file: modelFile, mmproj_file: mmprojFile })
        });
        if (!response.ok) throw new Error(await response.text());
        const result = await response.json();
        state.downloadId = result.id;
        $("#model-download-progress").hidden = false;
        await refreshDownloadStatus();
    } catch (error) {
        $("#model-browser-status").textContent = `Could not start model download: ${error.message}`;
    }
}

async function refreshDownloadStatus() {
    try {
        const response = await fetch("/api/models/download/status");
        if (!response.ok) throw new Error(await response.text());
        const job = await response.json();
        state.downloadId = job.id;
        renderDownloadStatus(job);
        if (job.status === "completed" && state.lastDownloadStatus !== "completed") {
            await loadInstalledModels();
            state.modelBrowserLoaded = false;
        }
        state.lastDownloadStatus = job.status;
    } catch (error) {
        $("#model-browser-status").textContent = `Could not check download status: ${error.message}`;
    }
}

function renderDownloadStatus(job) {
    const panel = $("#model-download-progress");
    if (job.status === "idle") { panel.hidden = true; return; }
    if (["completed", "cancelled", "failed"].includes(job.status) && job.id && state.dismissedDownloadId === job.id) {
        panel.hidden = true;
        return;
    }
    panel.hidden = false;
    $("#download-progress-title").textContent = `${job.repo_id || "Model"} · ${job.status}`;
    const percent = job.total_bytes > 0 ? Math.min(100, Math.floor(job.downloaded_bytes * 100 / job.total_bytes)) : 0;
    $("#download-progress-percent").textContent = job.total_bytes > 0 ? `${percent}% · ${formatBytes(job.downloaded_bytes)} / ${formatBytes(job.total_bytes)}` : formatBytes(job.downloaded_bytes);
    const progress = $("#model-download-progress-bar");
    progress.value = percent; progress.removeAttribute("value");
    if (job.total_bytes > 0) progress.value = percent;
    $("#download-progress-detail").textContent = `${job.detail}${job.current_file ? ` · ${job.current_file}` : ""}`;
    const active = ["downloading", "paused", "cancelling"].includes(job.status);
    $("#pause-model-download-btn").hidden = !active || job.status !== "downloading";
    $("#resume-model-download-btn").hidden = job.status !== "paused";
    $("#cancel-model-download-btn").hidden = !active;
    $("#dismiss-model-download-btn").hidden = active;
    if (!active && job.status !== "completed") $("#model-browser-status").textContent = job.detail;
    if (job.status === "completed") $("#model-browser-status").textContent = job.detail;
}

async function controlModelDownload(action) {
    if (!state.downloadId) return;
    try {
        const response = await fetch("/api/models/download/control", {
            method: "POST", headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ id: state.downloadId, action })
        });
        if (!response.ok) throw new Error(await response.text());
        renderDownloadStatus(await response.json());
    } catch (error) { $("#download-progress-detail").textContent = `Download action failed: ${error.message}`; }
}

async function loadInstalledModels() {
    try {
        const response = await fetch("/api/models/installed");
        if (!response.ok) throw new Error(await response.text());
        state.installedModels = await response.json();
        renderInstalledModels();
    } catch (error) {
        $("#installed-models").textContent = `Could not read downloaded models: ${error.message}`;
    }
}

function renderInstalledModels() {
    const container = $("#installed-models"); container.replaceChildren();
    if (!state.installedModels.length) {
        const empty = document.createElement("p"); empty.className = "field-hint"; empty.textContent = "No models have been downloaded yet."; container.append(empty); return;
    }
    state.installedModels.forEach(model => {
        const row = document.createElement("article"); row.className = "installed-model";
        const main = document.createElement("div"); main.className = "installed-model-main";
        const title = document.createElement("h4"); title.textContent = model.repo_id;
        const meta = document.createElement("div"); meta.className = "installed-model-meta";
        meta.append(modelMetaItem(`${model.files.length} GGUF ${model.files.length === 1 ? "file" : "files"}`));
        meta.append(modelMetaItem(formatBytes(model.size)));
        meta.append(modelMetaItem(model.active ? `Loaded: ${model.model_file || "model"}` : "Not loaded"));
        const externalServer = !!state.serverStatus && state.serverStatus.state !== "offline" && !state.serverStatus.managed;
        if (model.active) meta.append(modelMetaItem("Stop or switch models before deleting."));
        else if (externalServer) meta.append(modelMetaItem("Stop the external server before deleting downloaded models."));
        main.append(title, meta);
        const actions = document.createElement("div"); actions.className = "installed-model-actions";
        const run = document.createElement("button"); run.type = "button"; run.className = "primary-button";
        const serverRunning = state.serverStatus && state.serverStatus.state !== "offline";
        run.textContent = model.active ? "Stop llama.cpp" : serverRunning ? "Switch to this model" : "Start this model";
        run.disabled = !model.model_file || (serverRunning && !state.serverStatus.managed);
        run.addEventListener("click", () => model.active ? stopManagedServer() : startInstalledModel(model));
        const remove = document.createElement("button"); remove.type = "button"; remove.className = "danger-button"; remove.textContent = "Delete";
        remove.disabled = model.active || externalServer;
        remove.title = model.active ? "Stop llama.cpp or switch models before deleting this model." : externalServer ? "Stop the external inference server before deleting models." : "Delete downloaded model files";
        remove.addEventListener("click", () => deleteInstalledModel(model));
        actions.append(run, remove); row.append(main, actions); container.append(row);
    });
}

async function startInstalledModel(model) {
    try {
        if (state.serverStatus?.state !== "offline") {
            if (!state.serverStatus?.managed) throw new Error("Stop the externally managed llama.cpp server before switching models.");
            await stopManagedServer();
        }
        const response = await fetch("/api/runtime-server/start", {
            method: "POST", headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ repo_id: model.repo_id, model_file: model.model_file, mmproj_file: model.mmproj_file })
        });
        if (!response.ok) throw new Error(await response.text());
        const result = await response.json(); $("#model-browser-status").textContent = result.detail;
        await refreshServerStatus();
    } catch (error) { $("#model-browser-status").textContent = `Could not start model: ${error.message}`; }
}

async function stopManagedServer() {
    try {
        const response = await fetch("/api/runtime-server/stop", { method: "POST" });
        if (!response.ok) throw new Error(await response.text());
        const result = await response.json(); $("#model-browser-status").textContent = result.detail;
        await refreshServerStatus(); await loadInstalledModels();
    } catch (error) { $("#model-browser-status").textContent = `Could not stop llama.cpp: ${error.message}`; }
}

async function deleteInstalledModel(model) {
    if (!window.confirm(`Delete all downloaded files for ${model.repo_id}? This cannot be undone.`)) return;
    try {
        const response = await fetch("/api/models/delete", {
            method: "DELETE", headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ repo_id: model.repo_id })
        });
        if (!response.ok) throw new Error(await response.text());
        $("#model-browser-status").textContent = `${model.repo_id} was deleted from the models folder.`;
        await loadInstalledModels(); state.modelBrowserLoaded = false;
    } catch (error) { $("#model-browser-status").textContent = `Could not delete model: ${error.message}`; }
}

async function refreshServerStatus() {
    try {
        const response = await fetch("/api/runtime-server/status");
        if (!response.ok) throw new Error(await response.text());
        const status = await response.json(); renderServerStatus(status);
    } catch (error) {
        renderServerStatus({ state: "offline", detail: "Strawberry Social cannot reach its local status endpoint." });
    }
}

function renderServerStatus(status) {
    const previousKey = `${state.serverStatus?.state}:${state.serverStatus?.managed}:${state.serverStatus?.active_model?.repo_id}`;
    state.serverStatus = status;
    const stateName = ["running", "starting", "offline"].includes(status.state) ? status.state : "offline";
    $("#server-status-indicator").dataset.state = stateName;
    $(".workspace-statusbar").dataset.state = stateName;
    $("#server-status-text").textContent = stateName.charAt(0).toUpperCase() + stateName.slice(1);
    $("#server-status-detail").textContent = status.active_model
        ? `${status.active_model.repo_id} · ${status.active_model.model_file}`
        : status.detail || "";
    const currentKey = `${status.state}:${status.managed}:${status.active_model?.repo_id}`;
    if (previousKey !== currentKey) loadInstalledModels();
}

function formatParameterCount(count, estimated = false) {
    if (!Number.isFinite(count) || count <= 0) return "Parameter count unavailable";
    const prefix = estimated ? "~" : "";
    if (count >= 1e9) return `${prefix}${(count / 1e9).toFixed(count % 1e9 === 0 ? 0 : 1)}B parameters`;
    if (count >= 1e6) return `${prefix}${(count / 1e6).toFixed(count % 1e6 === 0 ? 0 : 1)}M parameters`;
    return `${prefix}${formatCount(count)} parameters`;
}

function formatCount(count) { return new Intl.NumberFormat().format(count); }
function formatModelDate(value) {
    if (!value) return "date unavailable";
    const date = new Date(value); return Number.isNaN(date.valueOf()) ? value : date.toLocaleDateString();
}
function quantizationRank(value) {
    const ranks = ["Q4_K_M", "Q5_K_M", "Q4_K_S", "Q5_K_S", "Q4_0", "Q5_0", "Q6_K", "Q8_0", "Q3_K_M", "Q3_K_S", "Q2_K", "F16", "BF16", "F32"];
    const index = ranks.indexOf(value); return index < 0 ? ranks.length + 1 : index;
}

async function loadRuntimeInfo(retry = false) {
    if (state.runtimeInfoLoading || (state.runtimeInfoLoaded && !retry)) return;
    state.runtimeInfoLoading = true;
    const release = $("#runtime-release");
    const status = $("#runtime-status");
    const retryButton = $("#retry-runtime-info");
    retryButton.hidden = true;
    release.textContent = "Checking GitHub…";
    status.textContent = "";
    try {
        const response = await fetch("/api/runtime-info");
        if (!response.ok) throw new Error(await response.text());
        const info = await response.json();
        $("#runtime-platform").textContent = info.platform || "Unknown";
        $("#auto-download-runtime").checked = !!info.auto_download_runtime;
        $("#runtime-asset").textContent = info.asset_name
            ? `${info.asset_name} (${formatBytes(info.asset_size)})`
            : "Not available for this platform yet";
        release.replaceChildren();
        if (info.latest_version && info.release_url) {
            const link = document.createElement("a");
            link.href = info.release_url; link.target = "_blank"; link.rel = "noopener noreferrer";
            link.textContent = info.latest_version; link.setAttribute("aria-label", `llama.cpp ${info.latest_version} release notes (opens in a new tab)`);
            release.append(link);
            if (!info.asset_name) {
                status.textContent = `Release ${info.latest_version} found. Built-in downloads are currently available on Linux x64 and ARM64 only.`;
                $("#download-runtime-btn").hidden = true;
            } else if (info.installed_runtime_tag === info.runtime_tag) {
                status.textContent = `Runtime ${info.runtime_tag} is installed and passed its startup check.`;
                $("#download-runtime-btn").hidden = true;
            } else {
                status.textContent = info.installed_runtime_tag
                    ? `Runtime ${info.installed_runtime_tag} is installed. A newer CPU build is available.`
                    : "No managed runtime is installed yet.";
                const downloadButton = $("#download-runtime-btn");
                downloadButton.textContent = `Download Linux CPU runtime (${formatBytes(info.asset_size)})`;
                downloadButton.hidden = false;
                if (info.auto_download_runtime) downloadRuntime(true);
            }
        } else {
            release.textContent = "No stable vX.Y.Z release found.";
            status.textContent = "The latest release list did not contain a matching stable version tag.";
        }
        state.runtimeInfoLoaded = true;
    } catch (error) {
        console.error("Could not retrieve llama.cpp release information:", error);
        release.textContent = "Unavailable";
        status.textContent = "Could not retrieve release information. Check your network connection and try again.";
        retryButton.hidden = false;
    } finally { state.runtimeInfoLoading = false; }
}

async function downloadRuntime(automatic) {
    const button = $("#download-runtime-btn");
    const retry = $("#retry-runtime-info");
    button.disabled = true;
    retry.hidden = true;
    $("#runtime-status").textContent = automatic
        ? "Automatically downloading the approved runtime update…"
        : "Downloading, verifying, extracting, and checking llama-server. This may take a few minutes…";
    try {
        const response = await fetch("/api/runtime-info/download", { method: "POST" });
        if (!response.ok) throw new Error(await response.text());
        const result = await response.json();
        $("#runtime-status").textContent = `Installed and verified ${result.runtime_tag}. The runtime is ready for model setup.`;
        state.runtimeInfoLoaded = false;
        await loadRuntimeInfo(true);
    } catch (error) {
        console.error("Runtime installation failed:", error);
        $("#runtime-status").textContent = `Download or verification failed: ${error.message}`;
        button.textContent = "Retry runtime download";
        button.hidden = false;
        retry.hidden = true;
    } finally { button.disabled = false; }
}

function formatBytes(bytes) {
    if (!Number.isFinite(bytes) || bytes <= 0) return "size unavailable";
    const units = ["B", "KB", "MB", "GB"];
    let value = bytes; let index = 0;
    while (value >= 1024 && index < units.length - 1) { value /= 1024; index += 1; }
    return `${value.toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
}

function applyTheme(theme) {
    document.body.classList.toggle("light-theme", theme === "light");
    const select = $("#theme-select"); if (select) select.value = theme === "light" ? "light" : "dark";
}

function bindCharacterDrawer() {
    $("#profile-btn").addEventListener("click", openCharacterDrawer);
    $("#close-drawer-btn").addEventListener("click", closeCharacterDrawer);
    $("#profile-overlay").addEventListener("click", closeCharacterDrawer);
}
function openCharacterDrawer() {
    const card = state.currentCard; if (!card) return;
    $("#drawer-name").textContent = card.name || "Character details";
    const body = $("#drawer-body-content"); body.replaceChildren();
    if (card.avatar_url) { const image = document.createElement("img"); image.src = card.avatar_url; image.className = "profile-avatar"; image.alt = ""; body.append(image); }
    [ ["Creator", card.creator || "Unknown"], ["Description", card.description || "No description provided."], ["Personality", card.personality || "No personality defined."], ["Scenario", card.scenario || "No scenario defined."] ].forEach(([label, value]) => {
        const section = document.createElement("section"); section.className = "profile-section";
        const heading = document.createElement("h3"); heading.textContent = label;
        const text = document.createElement("p"); text.innerHTML = formatMessage(value); section.append(heading, text); body.append(section);
    });
    $("#profile-drawer").classList.add("open"); $("#profile-overlay").classList.add("open");
}
function closeCharacterDrawer() { $("#profile-drawer").classList.remove("open"); $("#profile-overlay").classList.remove("open"); }
