import {defineConfig} from '@playwright/test';
export default defineConfig({
 testDir:'tests/e2e',testMatch:'*.spec.mjs',fullyParallel:false,workers:1,
 timeout:45000,use:{headless:true,trace:'retain-on-failure',...(process.env.PLAYWRIGHT_CHANNEL?{channel:process.env.PLAYWRIGHT_CHANNEL}:{})},
 reporter:'list',outputDir:'agent-artifacts/playwright',
});
