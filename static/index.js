let ws;
const teamGridEl = document.getElementById("teamGrid");

function connect() {
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  ws = new WebSocket(`${protocol}//${window.location.host}/ws`);

  ws.onopen = () => {
    console.log("Connected to server");
  };

  ws.onmessage = (event) => {
    try {
      const data = JSON.parse(event.data);
      updateTeam(data.pokemon);
    } catch (error) {
      console.error("Error parsing message:", error);
    }
  };

  ws.onerror = (error) => {
    console.error("WebSocket error:", error);
  };

  ws.onclose = () => {
    console.log("Disconnected from server");

    // Attempt to reconnect after 2 seconds
    setTimeout(connect, 2000);
  };
}

function updateTeam(pokemon) {
  teamGridEl.innerHTML = "";

  for (let i = 0; i < 6; i++) {
    const pokemonName = pokemon[i].name || "";
    const pokemonNickname = pokemon[i].nickname || "";
    const isEmpty = !pokemonName;

    const card = document.createElement("div");
    card.className = `pokemon-card ${isEmpty ? "empty" : ""} fade-in`;
    card.style.animationDelay = `${i * 0.1}s`;

    const spriteContainer = document.createElement("div");
    spriteContainer.className = `sprite-container ${isEmpty ? "empty" : ""}`;

    if (!isEmpty) {
      const img = document.createElement("img");
      // Try multiple common sprite naming conventions
      img.src = `/sprites/${pokemonName}.png`;
      img.alt = pokemonName;
      img.onerror = function () {
        // Try with different extensions or formats if PNG fails
        if (this.src.endsWith(".png")) {
          this.src = `/sprites/${pokemonName}.gif`;
        } else if (this.src.endsWith(".gif")) {
          this.src = `/sprites/${pokemonName}.jpg`;
        } else {
          // If all formats fail, show placeholder
          this.style.display = "none";
          spriteContainer.classList.add("empty");
        }
      };
      spriteContainer.appendChild(img);
    }

    const nameEl = document.createElement("div");
    nameEl.className = `pokemon-name ${isEmpty ? "empty" : ""}`;
    nameEl.textContent = isEmpty
      ? "Empty Slot"
      : pokemonNickname || pokemonName;

    card.appendChild(spriteContainer);
    card.appendChild(nameEl);
    teamGridEl.appendChild(card);
  }
}

// Initialize connection
connect();
