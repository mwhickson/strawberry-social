let currentSessionId = null;
let currentActiveCharacter = null;

document.addEventListener("DOMContentLoaded", () => {
    loadCharacters();
    loadSessions();

    const chatForm = document.getElementById("chat-form");
    chatForm.addEventListener("submit", handleSendMessage);
});

async function loadCharacters() {
    try {
        const res = await fetch("/api/characters");
        const characters = await res.json();
        const rosterContainer = document.getElementById("character-list");

        if (characters.length === 0) {
            rosterContainer.innerHTML = `<div class="loading-text">No cards found in /characters. Drop a JSON or PNG card in!</div>`;
            return;
        }

        rosterContainer.innerHTML = "";
        characters.forEach(char => {
            const div = document.createElement("div");
            div.className = "roster-item";
            div.innerHTML = `<strong>${escapeHtml(char.name)}</strong><br><small>${escapeHtml(char.creator || 'Unknown creator')}</small>`;
            div.onclick = () => selectCharacter(char, div);
            rosterContainer.appendChild(div);
        });
    } catch (err) {
        console.error("Failed to load characters:", err);
    }
}

async function selectCharacter(char, element) {
    document.querySelectorAll(".roster-item").forEach(el => el.classList.remove("active"));
    element.classList.add("active");
    currentActiveCharacter = char;

    document.getElementById("current-chat-title").innerText = `Chatting with ${char.name}`;
    
    // Create a new session for this character
    try {
        const res = await fetch("/api/chat/sessions", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ character_name: char.name, title: `Chat with ${char.name}` })
        });
        const session = await res.json();
        currentSessionId = session.id;

        loadSessions();
        loadMessages(currentSessionId);

        document.getElementById("message-input").disabled = false;
        document.getElementById("send-btn").disabled = false;
    } catch (err) {
        console.error("Failed to create session:", err);
    }
}

async function loadSessions() {
    try {
        const res = await fetch("/api/chat/sessions");
        const sessions = await res.json();
        const sessionContainer = document.getElementById("session-list");

        if (sessions.length === 0) return;

        sessionContainer.innerHTML = "";
        sessions.forEach(sess => {
            const div = document.createElement("div");
            div.className = `session-item ${sess.id === currentSessionId ? 'active' : ''}`;
            div.innerHTML = `<strong>${escapeHtml(sess.title)}</strong><br><small>${sess.character_name}</small>`;
            div.onclick = () => switchSession(sess.id, div);
            sessionContainer.appendChild(div);
        });
    } catch (err) {
        console.error("Failed to load sessions:", err);
    }
}

async function switchSession(sessionId, element) {
    document.querySelectorAll(".session-item").forEach(el => el.classList.remove("active"));
    element.classList.add("active");
    currentSessionId = sessionId;

    loadMessages(sessionId);
    document.getElementById("message-input").disabled = false;
    document.getElementById("send-btn").disabled = false;
}

async function loadMessages(sessionId) {
    try {
        const res = await fetch(`/api/chat/sessions/${sessionId}/messages`);
        const messages = await res.json();
        const container = document.getElementById("chat-messages");

        if (messages.length === 0) {
            container.innerHTML = `<div class="welcome-banner"><h3>Start the conversation!</h3><p>Say hello to your character.</p></div>`;
            return;
        }

        container.innerHTML = "";
        messages.forEach(msg => {
            appendMessageToUI(msg.role, msg.content);
        });
    } catch (err) {
        console.error("Failed to load messages:", err);
    }
}

async function handleSendMessage(e) {
    e.preventDefault();
    if (!currentSessionId) return;

    const input = document.getElementById("message-input");
    const content = input.value.trim();
    if (!content) return;

    input.value = "";
    input.disabled = true;
    document.getElementById("send-btn").disabled = true;

    // Optimistically append user message
    appendMessageToUI("user", content);

    try {
        const res = await fetch(`/api/chat/sessions/${currentSessionId}/messages`, {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ content })
        });
        const data = await res.json();
        
        // Append assistant response
        appendMessageToUI("assistant", data.assistant_message.content);
    } catch (err) {
        console.error("Failed to send message:", err);
        appendMessageToUI("assistant", "⚠️ Error: Failed to communicate with local LLM backend.");
    } finally {
        input.disabled = false;
        document.getElementById("send-btn").disabled = false;
        input.focus();
    }
}

function appendMessageToUI(role, text) {
    const container = document.getElementById("chat-messages");
    
    // Remove welcome banner if present
    const banner = container.querySelector(".welcome-banner");
    if (banner) banner.remove();

    const div = document.createElement("div");
    div.className = `message ${role}`;
    div.innerText = text;
    container.appendChild(div);
    container.scrollTop = container.scrollHeight;
}

function escapeHtml(str) {
    return str.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}