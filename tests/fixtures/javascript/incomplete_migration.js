// Planted incomplete migration: legacy_validate + validate coexist
function legacy_validate(data) {
    return typeof data === 'object';
}

function validate(data) {
    return data !== null && typeof data === 'object' && !Array.isArray(data);
}

function processOldWay(input) {
    if (legacy_validate(input)) {
        return input;
    }
}

function processNewWay(input) {
    if (validate(input)) {
        return input;
    }
}
