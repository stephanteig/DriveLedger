import { PageHeader, Panel, Empty } from "../components/ui";

export default function Placeholder({ title }: { title: string }) {
  return (
    <>
      <PageHeader title={title} />
      <Panel>
        <Empty>Coming in the next build. The CLI and MCP server already support this.</Empty>
      </Panel>
    </>
  );
}
