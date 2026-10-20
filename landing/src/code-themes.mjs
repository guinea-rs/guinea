function theme(type, palette) {
	return {
		name: `guinea-${type}`,
		type,
		colors: {
			'editor.background': palette.background,
			'editor.foreground': palette.text,
		},
		tokenColors: [
			{ scope: ['comment'], settings: { foreground: palette.comment } },
			{ scope: ['keyword', 'storage'], settings: { foreground: palette.accent } },
			{ scope: ['string'], settings: { foreground: palette.string } },
			{ scope: ['constant'], settings: { foreground: palette.number } },
			{ scope: ['entity.name.function', 'support.function'], settings: { foreground: palette.function } },
			{ scope: ['entity.name.type', 'entity.name.namespace', 'support.type', 'support.class'], settings: { foreground: palette.type } },
			{ scope: ['variable'], settings: { foreground: palette.text } },
			{ scope: ['variable.language'], settings: { foreground: palette.accent } },
			{ scope: ['punctuation'], settings: { foreground: palette.punctuation } },
		],
	};
}

export const codeThemes = [
	theme('dark', {
		background: '#1f2a2e',
		text: '#e2e8e6',
		comment: '#93a5aa',
		accent: '#f2b078',
		string: '#bccd96',
		number: '#e7a17e',
		function: '#e4c68c',
		type: '#8fcbe0',
		punctuation: '#b5c2c4',
	}),
	theme('light', {
		background: '#f7f8f3',
		text: '#2e3a3e',
		comment: '#596e75',
		accent: '#9b481c',
		string: '#4e651f',
		number: '#a34727',
		function: '#805817',
		type: '#206277',
		punctuation: '#45555a',
	}),
];
