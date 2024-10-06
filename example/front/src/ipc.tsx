import { pyInvoke, fromJson } from "tauri-plugin-pytauri-api";

export interface Person {
    name: string;
}

export interface Greeting {
    message: string;
}

export async function greet(person: Person): Promise<Greeting> {
    // NOTE: DO NOT use `greet.name` as `funcName` parameter,
    // rollup will change the identifier of `greet` function.
    // see: <https://github.com/rollup/rollup/issues/1914>
    const response = await pyInvoke("greet", person);
    return fromJson(response);
}
