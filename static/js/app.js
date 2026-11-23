const PROFILE_STORAGE_KEY = "strawberry-social-profiles-v1";
const ACTIVE_PROFILE_KEY = "strawberry-social-active-profile-v1";
const THEME_KEY = "strawberry-social-theme-v1"; // Used only once to migrate the previous browser preference.

const state = {
    config: {}, characters: [], sessions: [], profiles: [], activeProfileId: null,
    currentSessionId: null, currentCharacter: null, currentCard: null, editingProfileId: null,
    runtimeInfoLoaded: false, runtimeInfoLoading: false
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
    bindCharacterDrawer();
    await loadProfiles();
    applyTheme(state.theme || "dark");
    await Promise.all([loadConfig(), loadCharacters(), loadSessions()]);
    renderProfiles();
}

function bindNavigation() {
    $$(".nav-item").forEach(button => button.addEventListener("click", () => navigate(button.dataset.view)));
    window.addEventListener("hashchange", () => navigate(location.hash.slice(1) || "chat", false));
    const initial = location.hash.slice(1);
    if (["chat", "profiles", "settings"].includes(initial)) navigate(initial, false);
}

function navigate(view, updateHash = true) {
    if (!["chat", "profiles", "settings"].includes(view)) view = "chat";
    $$(".app-view").forEach(section => {
        const active = section.id === `view-${view}`;
        section.hidden = !active;
        section.classList.toggle("active", active);
    });
    $$(".nav-item").forEach(button => button.classList.toggle("active", button.dataset.view === view));
    if (updateHash && location.hash !== `#${view}`) history.replaceState(null, "", `#${view}`);
    if (view === "settings" && !state.runtimeInfoLoaded) loadRuntimeInfo();
}

async function loadConfig() {
    try {
        const response = await fetch("/api/config");
        if (!response.ok) throw new Error(await response.text());
        state.config = await response.json();
        $("#api-base-url").value = state.config.api_base_url || "";
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
        release.replaceChildren();
        if (info.latest_version && info.release_url) {
            const link = document.createElement("a");
            link.href = info.release_url; link.target = "_blank"; link.rel = "noopener noreferrer";
            link.textContent = info.latest_version; link.setAttribute("aria-label", `llama.cpp ${info.latest_version} release notes (opens in a new tab)`);
            release.append(link);
            status.textContent = "Release information retrieved.";
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
