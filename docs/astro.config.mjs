import { defineConfig } from 'astro/config';
import { unified } from '@astrojs/markdown-remark';
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

const pages = ['README', 'getting-started', 'plugin-management', 'cli-reference', 'architecture', 'development', 'troubleshooting', 'changelog'];
export default defineConfig({
  site: 'https://natlabrockies.github.io',
  base: '/r2x-cli/',
  markdown: { processor: unified(), remarkPlugins: [siteLinks] },
  redirects: Object.fromEntries(pages.map(page => [`/${page}.md`, `/r2x-cli/${page === 'README' ? '' : page + '/'}`])),
  integrations: [starlight({
    title: 'r2x-cli',
    description: 'Install Python plugins and run model interoperability pipelines.',
    social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/NatLabRockies/r2x-cli' }],
    editLink: { baseUrl: 'https://github.com/NatLabRockies/r2x-cli/edit/main/' },
    sidebar: [
      { label: 'Tutorials', items: [{ label: 'Getting started', link: '/getting-started/' }] },
      { label: 'How-to guides', items: [{ label: 'Manage plugins', link: '/plugin-management/' }, { label: 'Validate a workspace', link: '/getting-started/#validate-and-run-a-pipeline' }] },
      { label: 'Reference/API', items: [{ label: 'CLI reference', link: '/cli-reference/' }] },
      { label: 'Explanation', items: [{ label: 'Architecture', link: '/architecture/' }] },
      { label: 'Troubleshooting', items: [{ label: 'Diagnose failures', link: '/troubleshooting/' }] },
      { label: 'Changelog', items: [{ label: 'Release history and versions', link: '/changelog/' }] },
      { label: 'Contributors', items: [{ label: 'Development', link: '/development/' }] },
    ],
  })],
});
