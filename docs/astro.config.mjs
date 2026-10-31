import { defineConfig } from 'astro/config';
import { unified } from '@astrojs/markdown-remark';
import mermaid from 'astro-mermaid';
import starlight from '@astrojs/starlight';

// Keep GitHub Markdown links usable after rendering under the package basepath.
function siteLinks() {
  return tree => {
    function visit(node) {
      if (node.type === 'link' && /^[\w-]+\.md(?:#.*)?$/.test(node.url)) {
        const [file, anchor] = node.url.split('#');
        const slug = file === 'README.md' ? '' : file.slice(0, -3) + '/';
        node.url = `/r2x-cli/${slug}${anchor ? '#' + anchor : ''}`;
      }
      node.children?.forEach(visit);
    }
    visit(tree);
  };
}

const pages = ['README', 'getting-started', 'plugin-management', 'cli-reference', 'architecture', 'development', 'troubleshooting', 'changelog', 'inspect-artifacts', 'installation', 'environment', 'verify-installation'];
export default defineConfig({
  site: 'https://natlabrockies.github.io',
  base: '/r2x-cli/',
  markdown: { processor: unified({ remarkPlugins: [siteLinks] }) },
  redirects: Object.fromEntries(pages.map(page => [`/${page}.md`, `/r2x-cli/${page === 'README' ? '' : page + '/'}`])),
  integrations: [mermaid({ enableLog: false, mermaidConfig: { theme: "base", fontFamily: "system-ui, sans-serif" } }), starlight({
    title: 'r2x-cli',
    logo: {
      light: './src/assets/r2x-logo-full-color.svg',
      dark: './src/assets/r2x-logo-white.svg',
      alt: 'R2X framework logo',
      replacesTitle: false,
    },
    customCss: ['./src/styles/brand.css'],
    description: 'Install Python plugins and run model interoperability pipelines.',
    social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/NatLabRockies/r2x-cli' }, { icon: 'code-branch', label: 'Browse source versions', href: 'https://github.com/NatLabRockies/r2x-cli/tags' }],
    editLink: { baseUrl: 'https://github.com/NatLabRockies/r2x-cli/edit/main/docs/' },
    sidebar: [
      { label: 'Getting started', items: [{ label: 'Installation', link: '/installation/' }, { label: 'Python environment setup', link: '/environment/' }, { label: 'Verify the CLI and first use', link: '/verify-installation/' }] },
      { label: 'Tutorials', items: [{ label: 'Run your first pipeline', link: '/getting-started/' }] },
      { label: 'How-to guides', items: [{ label: 'Manage plugins', link: '/plugin-management/' }, { label: 'Inspect and transfer artifacts', link: '/inspect-artifacts/' }, { label: 'Validate a workspace', link: '/getting-started/#read-the-real-pipeline' }] },
      { label: 'Reference/API', items: [{ label: 'CLI reference', link: '/cli-reference/' }] },
      { label: 'Explanation', items: [{ label: 'Architecture', link: '/architecture/' }] },
      { label: 'Troubleshooting', items: [{ label: 'Diagnose failures', link: '/troubleshooting/' }] },
      { label: 'Changelog', link: 'https://github.com/NatLabRockies/r2x-cli/blob/main/CHANGELOG.md' },
      { label: 'Contributors', items: [{ label: 'Development', link: '/development/' }] },
    ],
  })],
});
