# Project Specification: Ryzen Control Center (Rust)

## 1. System Architecture
To keep the application stable and secure, the project should follow a simple **Model-View-Controller (MVC)** pattern.

*   **Frontend (The View):** Built using `iced` or `egui`.
*   **State Manager (The Model):** A Rust `struct` that holds current temperatures, power limits, and fan points.
*   **System Wrapper (The Controller):** A dedicated module that executes `pkexec ryzenadj` commands or writes directly to `/sys/class/hwmon/`.



---

## 2. Feature Specifications

### A. ACPI Platform Profiles
This module manages the high-level power behavior of the firmware.
*   **Interface:** A "Radio Button" group or a Segmented Switch.
*   **Options:** `Quiet`, `Balanced`, `Performance`.
*   **Backend Implementation:**
    *   If using `asusctl`: Execute `asusctl profile -p <profile>`.
    *   Native Linux: Write to `/sys/firmware/acpi/platform_profile`.
*   **Validation:** On startup, the app must read the current profile to sync the UI state.

### B. AMD Power Limits (PPT)
Advanced power tuning for Ryzen processors requires passing specific milliwatt (mW) values to `ryzenadj`.
*   **Inputs:** Numeric input boxes with "Increment/Decrement" buttons and a "Sync" slider.
*   **Targets:**
    *   **PPT APU:** Sustain power limit.
    *   **PPT Fast:** Short-term burst limit.
    *   **PPT Slow:** Long-term burst limit.
*   **Logic:** Implement a safety check where $PPT_{APU} \le PPT_{Slow} \le PPT_{Fast}$ to prevent invalid configurations.

### C. Fan Curve Controller
A coordinate-based graph for mapping Temperature to Fan Speed.
*   **The Widget:** A 2D Canvas with $n$ draggable points.
*   **Shift Logic:** A "Global Offset" slider. 
    *   Moving this slider modifies all $x$ coordinates (Temperature) by a constant $\Delta$.
    *   Formula: $x_{new} = x_{original} + \Delta$.
*   **Hysteresis Support:** An input field to define the temperature "buffer" (e.g., 2°C) to prevent fans from rapidly spinning up and down at a specific threshold.

---

## 3. Data Flow & Execution

| Action | UI Component | Backend Action |
| :--- | :--- | :--- |
| **Change Profile** | Radio Button click | `spawn("asusctl", ["profile", "-p", "quiet"])` |
| **Adjust PPT** | Slider Release | `spawn("pkexec", ["ryzenadj", "--stapm-limit=35000", ...])` |
| **Shift Fan Curve** | Offset Slider | Recalculate all points and write to `hwmon` pwm files |

---

## 4. Technical Challenges to Solve

### Permission Escalation
Most of these tools require `root`.
*   **Approach:** Do not run the GUI as root (this breaks Wayland/X11 scaling and themes).
*   **Execution:** Wrap calls in `pkexec`. For fan curves, you may need to write a small **Polkit rule** to allow your app to write to specific `/sys/` paths without a password prompt every 5 seconds.

### The "Loop" Requirement
Unlike a standard app, this needs a background "Watcher":
1.  **Poll:** Every 1–2 seconds, fetch the current CPU Temp and Power Draw.
2.  **Update:** Refresh the live "dot" on the fan curve graph.
3.  **Safety:** If a temperature exceeds a "Critical" threshold (e.g., 95°C), the app should automatically trigger the `Performance` profile to ensure max cooling.
