wasmtime::component::bindgen!({
	path: "wit/plugin.wit",
	imports: { default: async | trappable },
	exports: { default: async },
});
