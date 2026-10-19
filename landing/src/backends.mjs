export const BACKENDS = {
	winui: 'WinUI',
	egui: 'egui',
};

const DEFAULT = 'winui';

function element(tagName, properties, children = []) {
	return {
		type: 'paragraph',
		data: { hName: tagName, hProperties: properties },
		children,
	};
}

function panel(node) {
	return element('div', { class: 'backend-panel', 'data-backend': node.name }, [...node.children]);
}

function panelOf(child) {
	if (child.type === 'containerDirective' && child.name in BACKENDS) {
		return { name: child.name, node: panel(child) };
	}
	const name = child.data?.hProperties?.['data-backend'];
	return name in BACKENDS ? { name, node: child } : null;
}

function tabs(names) {
	const buttons = names
		.map(
			(name) =>
				`<button type="button" role="tab" class="backend-tab" data-backend-tab="${name}">${BACKENDS[name]}</button>`,
		)
		.join('');
	return { type: 'html', value: `<div class="backend-tabs" role="tablist">${buttons}</div>` };
}

/** `::::backends` holding `:::winui` and `:::egui`: one example per backend, shown as tabs. */
export function backendTabs() {
	return {
		name: 'guinea-backend-tabs',
		containerDirective(node) {
			if (node.name in BACKENDS) {
				return panel(node);
			}
			if (node.name !== 'backends') {
				return;
			}

			const panels = node.children.map(panelOf).filter(Boolean);
			return element('div', { class: 'backends' }, [
				tabs(panels.map((panel) => panel.name)),
				...panels.map((panel) => panel.node),
			]);
		},
	};
}

export const chooseBackend = `(() => {
	const key = 'guinea-backend';
	let chosen = null;
	try { chosen = localStorage.getItem(key); } catch {}
	document.documentElement.dataset.backend = chosen || '${DEFAULT}';
	document.addEventListener('click', (event) => {
		const tab = event.target.closest('[data-backend-tab]');
		if (!tab) return;
		const backend = tab.dataset.backendTab;
		const before = tab.getBoundingClientRect().top;
		document.documentElement.dataset.backend = backend;
		window.scrollBy(0, tab.getBoundingClientRect().top - before);
		try { localStorage.setItem(key, backend); } catch {}
	});
})();`;
