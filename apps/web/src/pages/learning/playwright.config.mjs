import {defineConfig} from '@playwright/test';
export default defineConfig({testDir:'.',testMatch:'*.spec.mjs',workers:1,outputDir:'../../../../../agent-artifacts/learning-mock',use:{headless:true,channel:"chrome"},reporter:'list'});
