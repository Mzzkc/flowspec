async function loadPlugin(name) {
    const mod = await import('./plugins/' + name);
    return mod;
}

function main() {}
