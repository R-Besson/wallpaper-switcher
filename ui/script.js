const qs = (s) => document.querySelector(s);
const qsa = (s) => document.querySelectorAll(s);

const api = {
	invoke: window.__TAURI__?.core?.invoke || window.__TAURI__?.invoke || null,

	async get() {
		if (!this.invoke) return {
			api_key: "", delay: "1h", minimize_to_tray: true,
			run_on_startup: false, theme_color: "#FFDDDD", sources: []
		};
		return await this.invoke('get_config');
	},

	async save(config) {
		if (!this.invoke) {
			console.log("Mock Save:", config);
			return;
		}
		return await this.invoke('save_config', { config });
	}
};

const state = {
	config: {},

	merge(newConfig) {
		this.config = { ...this.config, ...newConfig };
		this.updateTheme();
		this.render();
	}
};

const utils = {
	debounce(fn, wait) {
		let t;
		return (...args) => {
			clearTimeout(t);
			t = setTimeout(() => fn(...args), wait);
		};
	},

	hexToRgb(hex) {
		const r = parseInt(hex.slice(1, 3), 16);
		const g = parseInt(hex.slice(3, 5), 16);
		const b = parseInt(hex.slice(5, 7), 16);
		return `${r}, ${g}, ${b}`;
	},

	getContrast(hex) {
		const r = parseInt(hex.slice(1, 3), 16);
		const g = parseInt(hex.slice(3, 5), 16);
		const b = parseInt(hex.slice(5, 7), 16);
		const yiq = ((r * 299) + (g * 587) + (b * 114)) / 1000;
		return yiq >= 128 ? '#0f1115' : '#ffffff';
	},

	showToast() {
		const toast = qs('#save-toast');
		toast.classList.add('visible');
		setTimeout(() => toast.classList.remove('visible'), 2000);
	}
};

const actions = {
	handleSave: utils.debounce(async () => {
		await api.save(state.config);
		utils.showToast();
	}, 1000),

	immediateSave() {
		api.save(state.config);
		utils.showToast();
	},

	updateSource(index, key, value) {
		state.config.sources[index][key] = value;
		if (key === 'name') this.handleSave();
		else this.immediateSave();
	},

	addSource() {
		state.config.sources.push({ name: "nature", kind: "unsplash", enabled: true });
		state.render();
		this.immediateSave();

		requestAnimationFrame(() => {
			const inputs = qsa('.source-input');
			const lastInput = inputs[inputs.length - 1];
			lastInput?.focus();
			lastInput?.scrollIntoView({ behavior: 'smooth' });
		});
	},

	removeSource(index) {
		const el = qsa('.source-item')[index];
		if (!el) return;

		el.style.opacity = '0';
		el.style.transform = 'translateX(20px)';

		el.addEventListener('transitionend', () => {
			state.config.sources.splice(index, 1);
			state.render();
			this.immediateSave();
		}, { once: true });
	},

	switchTab(tabName) {
		const target = qs(`#tab-${tabName}`);
		const navItem = qs(`.nav-item[data-tab="${tabName}"]`);

		qsa('.page').forEach(p => {
			p.classList.remove('active');
			p.style.opacity = '0';
			setTimeout(() => { if (!p.classList.contains('active')) p.style.display = 'none'; }, 200);
		});

		qsa('.nav-item').forEach(n => n.classList.remove('active'));

		target.style.display = 'block';
		requestAnimationFrame(() => {
			target.classList.add('active');
			target.style.opacity = '1';
		});

		navItem.classList.add('active');
	}
};

state.updateTheme = () => {
	const hex = state.config.theme_color || "#FFDDDD";
	const root = document.documentElement;

	root.style.setProperty('--primary', hex);
	root.style.setProperty('--on-primary', utils.getContrast(hex));
	root.style.setProperty('--primary-rgb', utils.hexToRgb(hex));

	qs('#hex-val').textContent = hex.toUpperCase();
	qs('#color-preview').style.backgroundColor = hex;
	qs('#theme-picker').value = hex;
};

state.render = () => {
	const list = qs('#source-list');
	const tmpl = qs('#tmpl-source-item');
	const fragment = document.createDocumentFragment();

	qs('#empty-state').style.display = state.config.sources.length ? 'none' : 'flex';
	list.innerHTML = '';

	state.config.sources.forEach((src, idx) => {
		const clone = tmpl.content.cloneNode(true);

		const root = clone.querySelector('.source-item');
		const icon = clone.querySelector('.source-icon');
		const input = clone.querySelector('.source-input');
		const toggle = clone.querySelector('.source-toggle');
		const delBtn = clone.querySelector('.delete');

		if (src.enabled) root.classList.add('active-source');

		icon.title = `Source: ${src.type || 'unsplash'}`;

		input.value = src.name;
		input.oninput = (e) => actions.updateSource(idx, 'name', e.target.value);

		toggle.checked = src.enabled;
		toggle.onchange = (e) => actions.updateSource(idx, 'enabled', e.target.checked);

		delBtn.onclick = () => actions.removeSource(idx);

		fragment.appendChild(clone);
	});

	list.appendChild(fragment);
};

(async function init() {
	try {
		const loaded = await api.get();
		state.merge(loaded);

		const bindInput = (id, key) => {
			const el = qs(id);
			el.value = state.config[key] || "";
			el.oninput = (e) => {
				state.config[key] = e.target.value;
				actions.handleSave();
			};
		};

		const bindCheck = (id, key) => {
			const el = qs(id);
			el.checked = state.config[key] || false;
			el.onchange = (e) => {
				state.config[key] = e.target.checked;
				actions.immediateSave();
			};
		};

		bindInput('#api-key', 'api_key');
		bindInput('#delay-input', 'delay');
		bindCheck('#tray-toggle', 'minimize_to_tray');
		bindCheck('#startup-toggle', 'run_on_startup');

		qs('#color-trigger').onclick = () => qs('#theme-picker').click();
		qs('#theme-picker').oninput = (e) => {
			state.config.theme_color = e.target.value;
			state.updateTheme();
			actions.handleSave();
		};

		qs('#nav-menu').addEventListener('click', (e) => {
			const item = e.target.closest('.nav-item');
			if (item) actions.switchTab(item.dataset.tab);
		});

		qs('#btn-add-source').onclick = () => actions.addSource();

	} catch (err) {
		console.error("Init failed", err);
	}
})();